package com.rrgmc.karaokemachine.remote;

import android.Manifest;
import android.annotation.TargetApi;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.Uri;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.webkit.JsResult;
import android.webkit.PermissionRequest;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

import java.io.File;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The whole UI: a WebView showing the server running inside this app.
 *
 * <p>There is no native interface to build, because the interface already exists — the same pages
 * the desktop remote serves, over loopback rather than the LAN.
 */
public final class MainActivity extends Activity {

    /** Where the remote's two databases go. Under {@code filesDir}, so app-private. */
    private static final String DATA_SUBDIR = "remote";

    private static final String PREFS = "km-remote";
    private static final String PREF_MACHINE = "machine";

    /**
     * Only a backstop against waiting forever; the real signal is {@link Native#failure()}.
     *
     * <p>Deliberately generous. The Go remote this follows used a twenty-second deadline and
     * reported a perfectly healthy server as dead, because its first run downloaded the whole
     * catalog before it started listening. That cannot happen here — the import runs behind the
     * pages — but a deadline that can be wrong is still worth not having.
     */
    private static final long WAIT_LIMIT_MS = 180_000L;

    /** Matches the page's own background, so the inset strips are not a dark band around it. */
    private static final int PAGE_BACKGROUND = 0xFFFFFFFF;

    private static final int TEXT_PRIMARY = 0xFF1B1F23;
    private static final int TEXT_MUTED = 0xFF6A737D;

    /** Closes the favorites folder sheet if it is open, and says whether it was. */
    private static final String DISMISS_SHEET =
            "(function(){var b=document.querySelector('.sheet-backdrop');"
                    + "if(!b){return false;}b.click();return true;})()";

    /** Android's own camera prompt, for the share page's scanner. */
    private static final int REQ_CAMERA = 1;

    /** Where to write a backup, chosen by the document picker. */
    private static final int REQ_SAVE = 2;

    /** Which file to restore from, chosen by the document picker. */
    private static final int REQ_OPEN = 3;

    /**
     * Pulls the filename out of a {@code Content-Disposition}.
     *
     * <p>The header already carries the name the server chose, and it is ASCII by construction —
     * `Document::filename` says why — so there is no RFC 5987 form to decode here.
     */
    private static final Pattern FILENAME =
            Pattern.compile("filename\\*?=(?:UTF-8'')?(\"[^\"]*\"|[^;]*)", Pattern.CASE_INSENSITIVE);

    /**
     * What to call a download whose header says nothing.
     *
     * <p>A fixed name beats an empty one: the picker shows the field blank and refuses to save until
     * something is typed, which reads as a broken app rather than a missing header.
     */
    private static final String FALLBACK_DOWNLOAD_NAME = "km-favorites.json";

    private final Handler main = new Handler(Looper.getMainLooper());

    private WebView web;
    /** The page's camera request, held while Android's own prompt is answered. */
    private PermissionRequest pendingCamera;
    /** The URL to fetch, held while the picker chooses where to put it. */
    private String pendingDownload;
    /** The page's file input, held while a file is picked. */
    private ValueCallback<Uri[]> pendingChooser;
    private Thread waiting;
    /** Watches for a machine while the "no machine found" screen is up. See {@link #watchForMachine}. */
    private Thread watching;
    private int bottomInsetPx;

    /**
     * Held while this app is on screen, so an mDNS browse can see anything.
     *
     * <p>This is the whole of the Android-specific half of discovery. Receiving multicast needs a
     * lock held for the duration of the browse; without one a browse returns nothing and reports
     * success, so a machine that is switched on and announcing itself is simply never found. Holding
     * it in Java for the foreground rather than taking it around each browse from Rust keeps the
     * native side free of upcalls, and makes the backgrounded case correct for nothing: with no lock
     * a browse finds nothing, and the server's recovery logic answers "stay where you are".
     */
    private WifiManager.MulticastLock multicastLock;

    // -- lifecycle --------------------------------------------------------------------------------

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        showStarting();
        // **The lock before the server, and the order is the whole point.** `Native.start` browses
        // for a machine during its second phase, and a browse with no multicast lock held sees
        // nothing and reports success. Taking the lock in `onStart` alone — which runs *after*
        // `onCreate` — meant the one browse that matters most, a first run with nothing remembered,
        // was the one guaranteed to fail. It is idempotent, so `onStart` taking it again is free.
        acquireMulticastLock();
        Native.start(dataDir(), savedMachine());
        waitForServer();
    }

    /**
     * Back on screen: take the multicast lock, and wake the page.
     *
     * <p>{@code onStart}/{@code onStop} rather than {@code onResume}/{@code onPause}, because a
     * permission prompt or a notification shade pauses an Activity without the user having gone
     * anywhere, and neither dropping discovery nor pausing a page underneath a dialog is right.
     */
    @Override
    protected void onStart() {
        super.onStart();
        acquireMulticastLock();
        if (web != null) {
            web.onResume();
            web.resumeTimers();
        }
    }

    @Override
    protected void onStop() {
        if (web != null) {
            web.onPause();
            // Process-wide rather than per-view, which is exactly right with one WebView and would
            // need care if a second ever appeared.
            web.pauseTimers();
        }
        releaseMulticastLock();
        super.onStop();
    }

    /**
     * The server may have started answering while a failure screen was up.
     *
     * <p>{@code web != null} is the test for "a page is showing", which is what this wants to know.
     */
    @Override
    protected void onResume() {
        super.onResume();
        if (web == null && Native.failure() == null) {
            waitForServer();
        }
    }

    /**
     * Lets go of the WebView, the waiting thread and the server.
     *
     * <p>The server is stopped only when this is a real exit rather than a rotation or a
     * reconfiguration, because stopping it would throw away the catalog import running behind the
     * pages. {@link Native#stop()} returns at once, which is what makes it safe here: the main
     * looper must not be blocked at the moment the user asked the app to go away.
     */
    @Override
    protected void onDestroy() {
        WebView doomed = web;
        bindWeb(null);
        discard(doomed);
        // Answered here as well, so neither outlives the renderer it belongs to — the same rule
        // `discard` already states for the WebView itself.
        if (pendingChooser != null) {
            pendingChooser.onReceiveValue(null);
            pendingChooser = null;
        }
        if (pendingCamera != null) {
            pendingCamera.deny();
            pendingCamera = null;
        }
        if (waiting != null) {
            waiting.interrupt();
            waiting = null;
        }
        stopWatching();
        if (isFinishing()) {
            Native.stop();
        }
        super.onDestroy();
    }

    // -- the server -------------------------------------------------------------------------------

    /**
     * Where the databases go, created if absent.
     *
     * <p>Passed down because the server has no default for it and must not have one: the crate that
     * would normally answer this on a desktop ships implementations for Linux, macOS, Windows and
     * the web and nothing else, so on Android it silently takes the Linux path — which depends on a
     * {@code $HOME} that is normally unset here and falls back to a directory nothing may write to.
     */
    private String dataDir() {
        File dir = new File(getFilesDir(), DATA_SUBDIR);
        if (!dir.exists() && !dir.mkdirs()) {
            Log.w(Native.TAG, "could not create " + dir);
        }
        return dir.getAbsolutePath();
    }

    private SharedPreferences prefs() {
        return getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    /** A machine address somebody typed, or null to discover one. */
    private String savedMachine() {
        String value = prefs().getString(PREF_MACHINE, null);
        if (value == null || value.trim().isEmpty()) {
            return null;
        }
        return value.trim();
    }

    private void saveMachine(String value) {
        SharedPreferences.Editor editor = prefs().edit();
        if (value == null || value.trim().isEmpty()) {
            editor.remove(PREF_MACHINE);
        } else {
            editor.putString(PREF_MACHINE, value.trim());
        }
        editor.apply();
    }

    /**
     * Waits for the server to answer, for as long as it is alive.
     *
     * <p>What means failure is {@link Native#failure()} being set, not a clock running out. The cap
     * is only a backstop.
     */
    private void waitForServer() {
        if (waiting != null && waiting.isAlive()) {
            return;
        }
        waiting = new Thread(() -> {
            long deadline = System.currentTimeMillis() + WAIT_LIMIT_MS;
            while (System.currentTimeMillis() < deadline) {
                int port = Native.port();
                if (port != 0) {
                    main.post(() -> onServerReady(port));
                    return;
                }
                if (Native.failure() != null) {
                    break;
                }
                try {
                    Thread.sleep(100);
                } catch (InterruptedException interrupted) {
                    return;
                }
            }
            main.post(() -> {
                String reason = Native.failure();
                showFailure(reason != null
                        ? reason
                        : getString(R.string.failed_slow));
            });
        }, "km-remote-wait");
        waiting.setDaemon(true);
        waiting.start();
    }

    /**
     * The server is answering. Show the page, or ask for a machine if there is nothing to show.
     *
     * <p>The second case is the one worth explaining. Not finding a machine is <b>not</b> a failure
     * here — the server starts perfectly well without one, and that is the whole point of an offline
     * remote — so the failure screen never appears for it. But a first run that found nothing has an
     * empty catalog and no way to fill it, and if the only place to type an address were the
     * failure screen it would be unreachable exactly when it is needed.
     */
    private void onServerReady(int port) {
        if (Native.machine() == null && Native.songs() <= 0) {
            showNoMachine(port);
            return;
        }
        showWeb(port);
    }

    // -- discovery --------------------------------------------------------------------------------

    private void acquireMulticastLock() {
        if (multicastLock != null && multicastLock.isHeld()) {
            return;
        }
        try {
            // The application context rather than this Activity: a lock outliving an Activity that
            // held it is a leak, and lint says so.
            WifiManager wifi =
                    (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wifi == null) {
                Log.w(Native.TAG, "no WifiManager; discovery will find nothing");
                return;
            }
            multicastLock = wifi.createMulticastLock("km-remote-discovery");
            multicastLock.setReferenceCounted(false);
            multicastLock.acquire();
        } catch (Exception e) {
            // Reported rather than fatal: without the lock, browsing finds nothing and everything
            // else — the song list, search, favorites — goes on working from this device's copy.
            Log.w(Native.TAG, "could not take the multicast lock; discovery will find nothing", e);
        }
    }

    private void releaseMulticastLock() {
        if (multicastLock != null && multicastLock.isHeld()) {
            multicastLock.release();
        }
    }

    // -- saving a backup, and picking one -------------------------------------------------------

    /**
     * Answers the page's camera request once Android has answered ours.
     *
     * <p><b>{@code results.length == 0} is the branch a reading of the happy path misses</b>, and it
     * is the one that hangs the scanner: that is how Android reports the prompt being *dismissed* —
     * by tapping outside it — rather than answered either way. Denied is the right answer to it.
     *
     * <p>A second denial is permanent: Android stops showing the prompt, and
     * {@code shouldShowRequestPermissionRationale} going false is how that is detectable. The page
     * says the camera was refused either way; the toast is what names the way back.
     */
    @Override
    public void onRequestPermissionsResult(
            int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        if (requestCode != REQ_CAMERA) {
            return;
        }
        PermissionRequest request = pendingCamera;
        pendingCamera = null;
        if (request == null) {
            return;
        }
        if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) {
            request.grant(new String[] {PermissionRequest.RESOURCE_VIDEO_CAPTURE});
            return;
        }
        request.deny();
        if (results.length > 0
                && !shouldShowRequestPermissionRationale(Manifest.permission.CAMERA)) {
            toast(getString(R.string.camera_in_settings));
        }
    }

    /**
     * Where a backup goes, and which file a restore reads.
     *
     * <p><b>Both cases answer on every path, cancellation included</b>, for the reason
     * {@link PageChromeClient} gives at length. A cancelled save has nothing to say and says
     * nothing; a cancelled pick has to hand {@code null} back, or the file input is dead for the
     * rest of the process.
     */
    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_SAVE) {
            String url = pendingDownload;
            pendingDownload = null;
            Uri target = data == null ? null : data.getData();
            if (resultCode != RESULT_OK || url == null || target == null) {
                return; // a cancelled save is not a failure
            }
            saveDownload(url, target);
            return;
        }
        if (requestCode == REQ_OPEN) {
            ValueCallback<Uri[]> callback = pendingChooser;
            pendingChooser = null;
            if (callback == null) {
                return;
            }
            // The framework's own helper: `null` on cancellation, which is the answer that leaves
            // the input able to ask again, and it handles a multi-select `clipData` for free.
            callback.onReceiveValue(
                    WebChromeClient.FileChooserParams.parseResult(resultCode, data));
        }
    }

    /**
     * Copies the export into the stream the picker handed back.
     *
     * <p>The URL is re-fetched rather than the bytes held, because a {@code DownloadListener} is
     * only ever given a URL — and this server is on loopback inside this process, with nothing to
     * authenticate against. The app never holds the file, which is why this needs no storage
     * permission and no {@code FileProvider}.
     *
     * <p><b>The copy loop is written out rather than using {@code InputStream.transferTo}</b>, which
     * looks like the obvious call and is API 33 against this application's {@code minSdk 26}. There
     * is no core-library desugaring here — it would need a dependency this project deliberately has
     * none of — and {@code lint { abortOnError = false }} means {@code NewApi} would not stop it. The
     * failure would be a {@code NoSuchMethodError} on exactly the old hardware {@code armeabi-v7a}
     * exists for, and on no device this has been tested on.
     */
    private void saveDownload(String url, Uri target) {
        Thread worker = new Thread(() -> {
            boolean saved = false;
            try {
                HttpURLConnection connection =
                        (HttpURLConnection) new java.net.URL(url).openConnection();
                connection.setConnectTimeout(10_000);
                connection.setReadTimeout(30_000);
                try (InputStream from = connection.getInputStream();
                        OutputStream into = getContentResolver().openOutputStream(target)) {
                    if (into != null) {
                        byte[] buffer = new byte[8192];
                        int read;
                        while ((read = from.read(buffer)) >= 0) {
                            into.write(buffer, 0, read);
                        }
                        saved = true;
                    }
                } finally {
                    connection.disconnect();
                }
            } catch (Exception failed) {
                Log.w(Native.TAG, "could not save the backup", failed);
            }
            boolean done = saved;
            main.post(() -> toast(getString(done ? R.string.saved : R.string.save_failed)));
        }, "km-remote-save");
        worker.setDaemon(true);
        worker.start();
    }

    /** The name the server chose for this download, or {@link #FALLBACK_DOWNLOAD_NAME}. */
    private static String downloadName(String disposition) {
        if (disposition != null) {
            Matcher found = FILENAME.matcher(disposition);
            if (found.find()) {
                String name = found.group(1);
                if (name != null) {
                    name = name.trim();
                    if (name.length() >= 2 && name.startsWith("\"") && name.endsWith("\"")) {
                        name = name.substring(1, name.length() - 1);
                    }
                    if (!name.isEmpty()) {
                        return name;
                    }
                }
            }
        }
        return FALLBACK_DOWNLOAD_NAME;
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show();
    }

    // -- the page ---------------------------------------------------------------------------------

    private void showWeb(int port) {
        // Nothing left watching for a machine: the page is what that watch exists to reach.
        stopWatching();
        // Never two of them: this is reached from a posted callback, so a second can in principle
        // arrive while the first is on screen — and the one being replaced holds a renderer and an
        // open event stream until it is told otherwise.
        discard(web);
        bindWeb(null);

        WebView view = new WebView(this);
        view.setLayoutParams(new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        view.getSettings().setJavaScriptEnabled(true);
        view.getSettings().setDomStorageEnabled(true);
        // **Required for the share page's scanner, and not for anything else here.** `scan.js`
        // calls `play()` after `getUserMedia` resolves, and by then the tap that started it no
        // longer counts as a user gesture — so with the default the preview never starts and the
        // symptom is a frozen first frame rather than an error.
        view.getSettings().setMediaPlaybackRequiresUserGesture(false);
        view.setBackgroundColor(PAGE_BACKGROUND);
        view.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView v, WebResourceRequest request) {
                Uri url = request == null ? null : request.getUrl();
                if (url == null || staysInside(url)) {
                    return false;
                }
                return openOutside(url);
            }

            @Override
            public void onPageFinished(WebView v, String url) {
                // Every page is a fresh document, so the measurement is re-applied rather than set
                // once.
                applySafeBottom();
            }
        });
        view.setWebChromeClient(new PageChromeClient());
        // **Saving a backup, and the header is the whole mechanism.** The export is an ordinary
        // loopback navigation carrying `Content-Disposition: attachment`; `staysInside` hands it
        // back to the WebView, which reads that header and arrives here instead of rendering the
        // file into the page.
        //
        // Do **not** "improve" the export into a `blob:` URL. `staysInside` returns true for any
        // non-http scheme, so a `blob:` would stay in the WebView, which cannot render one — and
        // nothing at all would happen.
        view.setDownloadListener((url, agent, disposition, mime, length) -> {
            pendingDownload = url;
            Intent save = new Intent(Intent.ACTION_CREATE_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    // From the listener's own argument rather than hardcoded: the server chose the
                    // content type and the header carries it, so reading it is both shorter and one
                    // fewer thing to keep in step.
                    .setType(mime == null || mime.isEmpty() ? "application/octet-stream" : mime)
                    .putExtra(Intent.EXTRA_TITLE, downloadName(disposition));
            try {
                startActivityForResult(save, REQ_SAVE);
            } catch (ActivityNotFoundException missing) {
                Log.w(Native.TAG, "no document picker on this device", missing);
                pendingDownload = null;
                toast(getString(R.string.no_picker));
            }
        });

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(PAGE_BACKGROUND);
        root.addView(view);
        // Padded at the top and sides but NOT at the bottom: the page's tab bar runs to the very
        // edge and pads its own contents clear of the gesture handle, the way a native bar does.
        // Padding here instead leaves a blank strip below it.
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            int left;
            int top;
            int right;
            int bottom;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                left = bars.left;
                top = bars.top;
                right = bars.right;
                bottom = bars.bottom;
            } else {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
            }
            v.setPadding(left, top, right, 0);
            bottomInsetPx = bottom;
            applySafeBottom();
            return insets;
        });

        setContentView(root);
        useDarkStatusBarIcons();
        bindWeb(view);
        view.loadUrl("http://127.0.0.1:" + port + "/");
    }

    /**
     * Answers the page's {@code alert()} and {@code confirm()}.
     *
     * <p><b>Without this the page loses controls, silently, and the app looks broken rather than
     * unfinished.</b> A {@code WebView} with no {@code WebChromeClient} does not merely skip the
     * dialog — {@code WebViewContentsClientAdapter} cancels the request outright, so
     * {@code confirm()} returns {@code false} with nothing on screen. htmx reads that as "the person
     * said no" and never sends the request, which is what took ✕ off the Queue tab: the button was
     * pressed, the queue did not change, and there was no dialog, no error and no log line to say
     * why. The ↑ and ↓ beside it worked, because they carry no {@code hx-confirm}.
     *
     * <p>Explicit dialogs rather than a bare {@code new WebChromeClient()}. The base class returns
     * {@code false} from both callbacks and the adapter then puts up a default dialog of its own,
     * which does work — but that is a detail of the Chromium WebView rather than a documented
     * promise, and the documented behavior of returning {@code false} is the silent {@code false}
     * this exists to stop.
     *
     * <p>Canceling on dismiss is what a browser does with the back gesture or a tap outside, and it
     * is the safe direction for both of the page's questions: taking a song out of the queue, and
     * taking one out of a favorites folder.
     *
     * <p><b>The camera and the file picker fail in the same shape as the dialog above, and worse.</b>
     * A {@link PermissionRequest} that is never answered leaves {@code getUserMedia} pending for
     * ever rather than failing, so the share page sits on "Starting the camera…" with no error and
     * no log line; a {@code ValueCallback} that is never called leaves {@code <input type="file">}
     * <em>permanently</em> dead, so the second attempt at restoring a backup does nothing at all.
     * Every branch of both callbacks below therefore answers, cancellation included.
     *
     * <p><b>Not static any more</b>, which it was: granting the camera needs
     * {@code requestPermissions} and picking a file needs {@code startActivityForResult}, and both
     * are the Activity's. Its lifetime is the WebView's and the WebView is released by
     * {@link #discard}, so holding the Activity here leaks nothing.
     */
    private final class PageChromeClient extends WebChromeClient {
        @Override
        public boolean onJsAlert(WebView view, String url, String message, JsResult result) {
            new AlertDialog.Builder(view.getContext())
                    .setMessage(message)
                    .setPositiveButton(android.R.string.ok, (dialog, which) -> result.confirm())
                    .setOnCancelListener(dialog -> result.cancel())
                    .show();
            return true;
        }

        @Override
        public boolean onJsConfirm(WebView view, String url, String message, JsResult result) {
            new AlertDialog.Builder(view.getContext())
                    .setMessage(message)
                    .setPositiveButton(android.R.string.ok, (dialog, which) -> result.confirm())
                    .setNegativeButton(android.R.string.cancel, (dialog, which) -> result.cancel())
                    .setOnCancelListener(dialog -> result.cancel())
                    .show();
            return true;
        }

        /**
         * Grants the camera to the share page's scanner, once Android itself has granted it to us.
         *
         * <p><b>Four branches and all four answer.</b> The unanswered case is not an error anywhere
         * and shows up only as a scanner that never starts and never fails.
         *
         * <p>The already-granted branch is what makes a second visit to the receive page instant:
         * the page auto-starts its camera, so without it Android's prompt would appear on every
         * visit. Anything that is not video capture is denied outright — the page asks for
         * {@code audio: false}, so a microphone request means the page is not ours, and this
         * application opens no input stream by design.
         */
        @Override
        public void onPermissionRequest(PermissionRequest request) {
            if (request == null) {
                return;
            }
            boolean wantsCamera = false;
            for (String resource : request.getResources()) {
                if (PermissionRequest.RESOURCE_VIDEO_CAPTURE.equals(resource)) {
                    wantsCamera = true;
                    break;
                }
            }
            if (!wantsCamera || !isOurOrigin(request.getOrigin())) {
                request.deny();
                return;
            }
            if (checkSelfPermission(Manifest.permission.CAMERA)
                    == PackageManager.PERMISSION_GRANTED) {
                request.grant(new String[] {PermissionRequest.RESOURCE_VIDEO_CAPTURE});
                return;
            }
            // One at a time. A second request arriving while the system dialog is up would
            // otherwise overwrite the first and leave it unanswered for ever.
            if (pendingCamera != null) {
                pendingCamera.deny();
            }
            pendingCamera = request;
            requestPermissions(new String[] {Manifest.permission.CAMERA}, REQ_CAMERA);
        }

        /**
         * Opens the document picker for the restore page's file input.
         *
         * <p><b>The intent is built here rather than by {@code params.createIntent()}, and that is
         * the opposite of what it looks like.</b> Doing it that way would turn the page's
         * {@code accept} list into an Android MIME filter — and Drive and Dropbox report their own
         * types for what they hold, so a backup that is plainly visible in the picker becomes
         * impossible to select. {@code *​/*} is deliberate; the {@code accept} list is for iOS,
         * which maps it to UTTypes and knows the extension.
         */
        @Override
        public boolean onShowFileChooser(
                WebView view,
                ValueCallback<Uri[]> callback,
                FileChooserParams params) {
            if (pendingChooser != null) {
                pendingChooser.onReceiveValue(null);
            }
            pendingChooser = callback;
            Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                    .addCategory(Intent.CATEGORY_OPENABLE)
                    .setType("*/*");
            try {
                startActivityForResult(pick, REQ_OPEN);
            } catch (ActivityNotFoundException missing) {
                Log.w(Native.TAG, "no document picker on this device", missing);
                pendingChooser = null;
                callback.onReceiveValue(null);
                toast(getString(R.string.no_picker));
                // **`true`, not `false`, and the framework's own words are the reason.** The SDK
                // documents the cancel as "call filePathCallback.onReceiveValue(null) and return
                // true", and says of the callback that it "must only be called if the
                // onShowFileChooser implementation returns true". Answering and then returning
                // `false` does both halves wrongly: it invokes a callback the contract forbids
                // invoking, and tells the WebView to fall back to default handling that would
                // invoke it a second time. On the one class of device this branch exists for -- no
                // document picker installed -- that is exactly where a dead file input comes from.
                return true;
            }
            return true;
        }
    }

    /**
     * Tells the page how much room the gesture bar needs.
     *
     * <p>A plain WebView reports no safe areas at all, so {@code env(safe-area-inset-bottom)} is
     * zero inside it and the stylesheet cannot work this out for itself. It already reads a
     * {@code --safe-bottom} custom property with that as its default, so an inline value on the
     * document element is all that is needed — and nothing in the page has to know it is running
     * here rather than in a browser.
     */
    private void applySafeBottom() {
        WebView view = web;
        if (view == null) {
            return;
        }
        int cssPx = (int) (bottomInsetPx / getResources().getDisplayMetrics().density);
        view.evaluateJavascript(
                "document.documentElement.style.setProperty('--safe-bottom', '" + cssPx + "px')",
                null);
    }

    /**
     * Whether a navigation belongs in this WebView.
     *
     * <p>Decided by host rather than by asserting that nothing ever links out. Nothing in the pages
     * does today; stating the general rule is what keeps that from becoming a trap the day one does.
     * Anything that is not http(s) stays, having no host to judge.
     */
    private boolean staysInside(Uri url) {
        String scheme = url.getScheme();
        if (!"http".equals(scheme) && !"https".equals(scheme)) {
            return true;
        }
        String host = url.getHost();
        return "127.0.0.1".equals(host) || "localhost".equals(host);
    }

    /**
     * Whether a page asking for the camera is our own server.
     *
     * <p><b>Stricter than {@link #staysInside}, and not a reuse of it.</b> That one answers
     * "does this navigation belong in the WebView?" and says yes to anything with no host to judge,
     * which is right for a `mailto:` and wrong for a camera: an opaque or absent origin would then
     * be granted. Here the host has to actually be loopback, and anything else — including null —
     * is refused.
     *
     * <p>The iOS shell makes the same check on a {@code WKSecurityOrigin} and the desktop window
     * cannot, having no origin handed to it; that one leans on the window only ever loading
     * {@code http://127.0.0.1:<port>/}. This is the one of the three where the origin is available
     * and worth asserting, so it is asserted rather than assumed.
     */
    private boolean isOurOrigin(Uri origin) {
        if (origin == null) {
            return false;
        }
        String host = origin.getHost();
        return "127.0.0.1".equals(host) || "localhost".equals(host);
    }

    /**
     * Hands a link to whatever app claims it, and says whether that worked.
     *
     * <p>No {@code FLAG_ACTIVITY_NEW_TASK}: started from an Activity the target lands on top of this
     * task, which is what makes the back gesture come straight back here. Returning false when
     * nothing can handle it lets the WebView load the page itself, so a tap that cannot leave still
     * does something.
     */
    private boolean openOutside(Uri url) {
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, url));
            return true;
        } catch (Exception e) {
            Log.w(Native.TAG, "nothing can open " + url + "; loading it here instead", e);
            return false;
        }
    }

    /**
     * Tears a WebView down for good.
     *
     * <p>{@code destroy()} is what releases the renderer and closes the event stream still open on
     * it; dropping the reference alone leaves both, and this app holds an event stream on every
     * page. It may not be called while the view is in a hierarchy, hence the removal first.
     */
    private void discard(WebView view) {
        if (view == null) {
            return;
        }
        ViewGroup parent = (ViewGroup) view.getParent();
        if (parent != null) {
            parent.removeView(view);
        }
        view.destroy();
    }

    // -- the screens that are not the page --------------------------------------------------------

    private void showStarting() {
        bindWeb(null);
        setContentView(message(getString(R.string.starting_title),
                getString(R.string.starting_body),
                null, null, null));
        useDarkStatusBarIcons();
    }

    /**
     * A first run that found no machine and has nothing to show.
     *
     * <p>The one screen the Go remote this follows does not have, and it is here because this server
     * behaves differently: it starts successfully with no machine, so the failure screen its address
     * field lives on would never appear.
     */
    private void showNoMachine(int port) {
        bindWeb(null);
        setContentView(message(getString(R.string.no_machine_title),
                getString(R.string.no_machine_body),
                getString(R.string.retry),
                getString(R.string.carry_on),
                () -> showWeb(port)));
        useDarkStatusBarIcons();
        watchForMachine(port);
    }

    /**
     * Moves off the "no machine found" screen by itself if one turns up.
     *
     * <p><b>Without this that screen is a dead end, and it was one.</b> The server goes on looking
     * in the background while it is showing — every twenty seconds, for as long as the machine is
     * unreachable — so on a real network the sequence is: the first browse at startup finds nothing,
     * this screen appears, and twenty seconds later the server quietly finds the machine, copies the
     * song list and connects. Measured on a phone, with the log showing all three of those and the
     * screen still saying nothing answered. A screen whose only sentence has become false, in front
     * of a working remote, with two buttons that both amount to "carry on".
     *
     * <p>So it watches instead of waiting to be tapped. The user can still type an address or press
     * on; this only takes over when the answer arrives on its own.
     *
     * <p><b>And for two milestones it did not work, which this paragraph used to claim it did.</b>
     * {@code Native.machine()} reported what the server found at startup and was never written
     * again — {@code Server.machine_watch} re-points its client every time it finds one, and none of
     * that reached the value this polls. So the loop below ran its full three minutes against a
     * constant, and the screen stayed exactly as dead as before the watcher was written. It went
     * unnoticed here because the multicast lock is taken in {@code onCreate}, so the first browse
     * usually succeeds on Android and this screen is rare; it was found on iOS, where the first
     * sweep loses its packets to the local-network prompt and the screen is therefore the common
     * first-run case, and then confirmed on a phone.
     *
     * <p>The fix is in {@code km-remote-host}, shared by both shells: it holds the live client
     * rather than a snapshot of the first answer. Nothing in this file changed.
     */
    private void watchForMachine(int port) {
        if (watching != null) {
            watching.interrupt();
        }
        watching = new Thread(() -> {
            long deadline = System.currentTimeMillis() + WAIT_LIMIT_MS;
            while (System.currentTimeMillis() < deadline) {
                if (Native.machine() != null) {
                    main.post(() -> {
                        // Only if this screen is still the one showing: the user may have pressed
                        // Carry on anyway, or typed an address and restarted the server, and neither
                        // should have the page pulled out from under it.
                        if (web == null) {
                            Log.i(Native.TAG, "a machine turned up; showing the remote");
                            showWeb(port);
                        }
                    });
                    return;
                }
                try {
                    Thread.sleep(500);
                } catch (InterruptedException interrupted) {
                    return;
                }
            }
        }, "km-remote-watch");
        watching.setDaemon(true);
        watching.start();
    }

    private void stopWatching() {
        if (watching != null) {
            watching.interrupt();
            watching = null;
        }
    }

    private void showFailure(String reason) {
        bindWeb(null);
        setContentView(message(getString(R.string.failed_title), reason,
                getString(R.string.retry), null, null));
        useDarkStatusBarIcons();
    }

    /**
     * The one settings surface there is: a title, an explanation, a machine address and up to two
     * buttons.
     *
     * <p>These screens are painted light explicitly rather than inheriting the device theme, which
     * is dark on a phone in dark mode — and the pages they hand over to are light whatever the phone
     * does. A dark screen handing over to a white page is a flash rather than a transition.
     */
    private View message(String title, String detail, String retryLabel,
                         String dismissLabel, Runnable onDismiss) {
        LinearLayout content = new LinearLayout(this);
        content.setOrientation(LinearLayout.VERTICAL);
        content.setGravity(Gravity.CENTER_VERTICAL);
        content.setPadding(64, 96, 64, 96);

        TextView heading = new TextView(this);
        heading.setText(title);
        heading.setTextSize(20f);
        heading.setTextColor(TEXT_PRIMARY);
        content.addView(heading);

        TextView body = new TextView(this);
        body.setText(detail);
        body.setTextSize(15f);
        body.setPadding(0, 32, 0, 0);
        body.setTextColor(TEXT_MUTED);
        content.addView(body);

        if (retryLabel != null) {
            EditText address = new EditText(this);
            address.setHint(getString(R.string.address_hint));
            address.setInputType(InputType.TYPE_TEXT_VARIATION_URI);
            address.setSingleLine(true);
            address.setText(savedMachine());
            content.addView(address);

            TextView hint = new TextView(this);
            hint.setText(getString(R.string.address_note));
            hint.setTextSize(12f);
            hint.setPadding(0, 16, 0, 0);
            hint.setTextColor(TEXT_MUTED);
            content.addView(hint);

            Button retry = new Button(this);
            retry.setText(retryLabel);
            retry.setOnClickListener(v -> {
                // Saved whether or not it is empty: clearing the box is how a pinned machine is
                // given up, and the server never wanders away from one it was told about.
                saveMachine(address.getText().toString());
                // Before the restart: the watcher is looking at state this is about to tear down,
                // and a stale one would post `showWeb` against a server that has already stopped.
                stopWatching();
                Native.stop();
                showStarting();
                Native.start(dataDir(), savedMachine());
                waitForServer();
            });
            content.addView(retry);
        }

        if (dismissLabel != null && onDismiss != null) {
            Button dismiss = new Button(this);
            dismiss.setText(dismissLabel);
            dismiss.setOnClickListener(v -> onDismiss.run());
            content.addView(dismiss);
        }

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(PAGE_BACKGROUND);
        root.addView(content);
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars());
                v.setPadding(bars.left, bars.top, bars.right, bars.bottom);
            } else {
                v.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                        insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            }
            return insets;
        });
        return root;
    }

    /**
     * The pages are light whatever the phone's theme is, so the status bar icons have to be dark or
     * they vanish into the white header.
     */
    private void useDarkStatusBarIcons() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            return;
        }
        WindowInsetsController controller = getWindow().getInsetsController();
        if (controller != null) {
            int light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                    | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
            controller.setSystemBarsAppearance(light, light);
        }
        getWindow().setStatusBarColor(Color.TRANSPARENT);
    }

    // -- back -------------------------------------------------------------------------------------

    /**
     * Back means one step back, and Android has two ways of saying it.
     *
     * <p>Targeting SDK 36 opts this app into predictive back, and from Android 13 that stops {@code
     * onBackPressed} being called and {@code KEYCODE_BACK} being dispatched at all — so with nothing
     * registered the system's default handling finishes the Activity, and backing out of an artist's
     * songs closes the app instead of returning to the list. There is no androidx dependency here to
     * cover both eras in one call, so both are wired by hand.
     *
     * <p>The page is asked first, which is why this is asynchronous: the favorites folder sheet is
     * a modal swapped in with no history entry behind it, so going back from it would leave the page
     * it sits on rather than shutting it. Clicking its backdrop is exactly what tapping outside the
     * sheet does, so how it closes stays defined in one place. Nothing may act before the script
     * answers — finishing early would exit the app with the sheet still on screen.
     */
    private void handleBack() {
        WebView view = web;
        if (view == null) {
            finish();
            return;
        }
        view.evaluateJavascript(DISMISS_SHEET, value -> {
            if (!"true".equals(value)) {
                if (view.canGoBack()) {
                    view.goBack();
                } else {
                    finish();
                }
            }
        });
    }

    /** The path for devices below API 33. Dead code from Android 13 on, which is what hides bugs. */
    @Override
    @Deprecated
    public void onBackPressed() {
        handleBack();
    }

    /**
     * The registered callback, held so it can be handed back to unregister.
     *
     * <p>A class of its own so that {@code android.window} is named nowhere in MainActivity's own
     * method descriptors: it is loaded the first time it is used, which only happens inside the
     * version check, so an API 26 device never has to resolve a package it does not have.
     */
    @TargetApi(Build.VERSION_CODES.TIRAMISU)
    private static final class WebBack {
        private final MainActivity activity;
        private final OnBackInvokedCallback callback;

        WebBack(MainActivity activity) {
            this.activity = activity;
            this.callback = activity::handleBack;
        }

        void register() {
            activity.getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback);
        }

        void unregister() {
            activity.getOnBackInvokedDispatcher().unregisterOnBackInvokedCallback(callback);
        }
    }

    private WebBack back;

    /**
     * Points back handling at the WebView while there is one, and lets it go otherwise.
     *
     * <p>Registration follows the WebView rather than its history: it stays in place at the first
     * page and the callback calls {@code finish()} there. Registering only while {@code canGoBack()}
     * would keep the system's back-to-home preview, but it would have to catch every history change
     * to stay honest — including the sheet, which arrives through a swap that reaches no
     * WebViewClient callback at all — and every moment the two disagreed would be this same bug,
     * arriving intermittently.
     */
    private void bindWeb(WebView view) {
        web = view;
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return;
        }
        if (view != null) {
            // Guarded, because a second showWeb must not leave two callbacks on the dispatcher.
            if (back == null) {
                back = new WebBack(this);
                back.register();
            }
        } else if (back != null) {
            back.unregister();
            back = null;
        }
    }
}
