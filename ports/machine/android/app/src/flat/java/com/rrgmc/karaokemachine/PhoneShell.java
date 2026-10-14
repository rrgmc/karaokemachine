package com.rrgmc.karaokemachine;

import android.annotation.SuppressLint;
import android.app.UiModeManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.util.Log;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;

/**
 * The controls a phone adds over the machine: its own remote, and a way to install a package.
 *
 * <p>On a phone the machine and the singer are one device. Switching to the remote application
 * takes the screen away, and the machine pauses when that happens. This shows the machine's own
 * remote over the screen instead, in the same window. The activity keeps both the screen and the
 * window focus, so SDL goes on drawing and nothing pauses. The page comes from this process over
 * loopback, so no network stands between the two.
 *
 * <p>Every view here is a plain Android view in the activity's own window. A popup menu or a
 * dialog is a window of its own and would take the focus, and SDL parks its thread when the focus
 * goes.
 *
 * <p>The headset flavour carries an empty twin, because its queue panel already does this job.
 */
final class PhoneShell {

    private static final String TAG = "KaraokeMachine";

    /** How long a page that did not load waits before it tries again. */
    private static final long RETRY_MS = 2000L;

    private static final int TEXT_SP = 14;
    private static final int PAD_X_DP = 14;
    private static final int PAD_Y_DP = 6;
    private static final int RADIUS_DP = 16;
    private static final int MARGIN_DP = 8;
    private static final int GAP_DP = 6;

    /** The strip beside the open remote that holds Close, so the button covers none of the page. */
    private static final int RAIL_DP = 88;

    /** The menu button over a song: visible enough to find, faint enough not to cover words. */
    private static final float IDLE_ALPHA = 0.55f;

    private static final String BUTTON_COLOR = "#1F6FEB";
    private static final String PAGE_COLOR = "#101418";

    private final MainActivity activity;
    private final FrameLayout layer;
    private final LinearLayout column;
    private final FrameLayout rail;
    private final Button menu;
    private final Button songs;
    private final Button install;

    /** The remote, built on first use and kept, so the page keeps its place between openings. */
    private WebView page;

    private boolean expanded;
    private boolean open;

    /**
     * Puts the controls over the machine on a phone or a tablet, and does nothing elsewhere.
     *
     * <p>A television has no touch screen and a remote in the hand already. Answers null where
     * nothing was added, and every caller checks.
     */
    static PhoneShell attach(MainActivity activity) {
        if (!activity.getPackageManager().hasSystemFeature(PackageManager.FEATURE_TOUCHSCREEN)) {
            return null;
        }
        UiModeManager modes = (UiModeManager) activity.getSystemService(Context.UI_MODE_SERVICE);
        if (modes != null
                && modes.getCurrentModeType() == Configuration.UI_MODE_TYPE_TELEVISION) {
            return null;
        }
        return new PhoneShell(activity);
    }

    private PhoneShell(MainActivity activity) {
        this.activity = activity;

        // One layer over the SDL surface, holding the page and the buttons. It handles no touch
        // itself, so a touch that misses every view in it reaches the machine below.
        layer = new FrameLayout(activity);

        menu = button(R.string.phone_menu);
        songs = button(R.string.phone_choose_songs);
        install = button(R.string.phone_install_package);
        menu.setOnClickListener(v -> expand(!expanded));
        songs.setOnClickListener(v -> show());
        install.setOnClickListener(v -> {
            expand(false);
            activity.choosePackage();
        });

        // Top centre, because the machine draws in both top corners: the song's title on the left,
        // and the key, the melody and the queue counter on the right.
        column = new LinearLayout(activity);
        column.setOrientation(LinearLayout.VERTICAL);
        column.setGravity(Gravity.CENTER_HORIZONTAL);
        column.addView(menu, item(0));
        column.addView(songs, item(dp(GAP_DP)));
        column.addView(install, item(dp(GAP_DP)));
        FrameLayout.LayoutParams top = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP | Gravity.CENTER_HORIZONTAL);
        top.topMargin = dp(MARGIN_DP);
        layer.addView(column, top);

        // The open remote's own controls fill its edges, so Close stands beside the page and not
        // on it. A phone held sideways has width to spare and little height.
        rail = new FrameLayout(activity);
        rail.setBackgroundColor(Color.parseColor(PAGE_COLOR));
        Button close = button(R.string.phone_close);
        close.setOnClickListener(v -> close());
        FrameLayout.LayoutParams inRail = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP | Gravity.CENTER_HORIZONTAL);
        inRail.topMargin = dp(MARGIN_DP);
        rail.addView(close, inRail);
        rail.setVisibility(View.GONE);
        layer.addView(rail, new FrameLayout.LayoutParams(
                dp(RAIL_DP), ViewGroup.LayoutParams.MATCH_PARENT, Gravity.END));

        expand(false);
        activity.addContentView(layer, new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
    }

    /**
     * Closes whatever is open, and answers whether there was anything.
     *
     * <p>Back closes the remote before it leaves the machine, because leaving pauses the song.
     */
    boolean back() {
        if (open) {
            close();
            return true;
        }
        if (expanded) {
            expand(false);
            return true;
        }
        return false;
    }

    /**
     * Shows or hides the two items under the menu button.
     *
     * <p>The owner can turn the machine's remote off, and then the page is not there to show. The
     * setting is read at each opening, because the remote's own settings page can change it.
     */
    private void expand(boolean on) {
        expanded = on;
        songs.setVisibility(on && Loopback.servesRemote(activity) ? View.VISIBLE : View.GONE);
        install.setVisibility(on ? View.VISIBLE : View.GONE);
        menu.setAlpha(on ? 1f : IDLE_ALPHA);
    }

    private void show() {
        expand(false);
        if (page == null) {
            page = page(Loopback.root(activity));
            FrameLayout.LayoutParams beside = new FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT);
            beside.rightMargin = dp(RAIL_DP);
            layer.addView(page, 0, beside);
        }
        page.setVisibility(View.VISIBLE);
        rail.setVisibility(View.VISIBLE);
        column.setVisibility(View.GONE);
        open = true;
    }

    private void close() {
        if (page != null) {
            page.setVisibility(View.GONE);
        }
        rail.setVisibility(View.GONE);
        column.setVisibility(View.VISIBLE);
        open = false;
    }

    /**
     * The machine's remote, retrying until the machine answers.
     *
     * <p>A link that leaves loopback opens in the browser, as it does in the remote's application.
     */
    @SuppressLint("SetJavaScriptEnabled")
    private WebView page(String address) {
        WebView web = new WebView(activity);
        web.setBackgroundColor(Color.parseColor(PAGE_COLOR));
        // The remote is htmx: it swaps fragments with script and keeps its preferences in storage.
        web.getSettings().setJavaScriptEnabled(true);
        web.getSettings().setDomStorageEnabled(true);
        web.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri url = request == null ? null : request.getUrl();
                if (url == null || staysInside(url)) {
                    return false;
                }
                try {
                    activity.startActivity(new Intent(Intent.ACTION_VIEW, url));
                    return true;
                } catch (RuntimeException e) {
                    Log.w(TAG, "nothing can open " + url + "; loading it here instead", e);
                    return false;
                }
            }

            @Override
            public void onReceivedError(
                    WebView view, WebResourceRequest request, WebResourceError error) {
                if (request.isForMainFrame()) {
                    view.postDelayed(() -> view.loadUrl(address), RETRY_MS);
                }
            }
        });
        web.loadUrl(address);
        return web;
    }

    /** Whether a navigation belongs in the page: loopback, or a scheme with no host to judge. */
    private static boolean staysInside(Uri url) {
        String scheme = url.getScheme();
        if (!"http".equals(scheme) && !"https".equals(scheme)) {
            return true;
        }
        String host = url.getHost();
        return "127.0.0.1".equals(host) || "localhost".equals(host);
    }

    private Button button(int text) {
        Button button = new Button(activity);
        button.setText(text);
        button.setAllCaps(false);
        button.setTextSize(TypedValue.COMPLEX_UNIT_SP, TEXT_SP);
        button.setTextColor(Color.WHITE);
        button.setPadding(dp(PAD_X_DP), dp(PAD_Y_DP), dp(PAD_X_DP), dp(PAD_Y_DP));
        button.setMinHeight(0);
        button.setMinimumHeight(0);
        GradientDrawable shape = new GradientDrawable();
        shape.setCornerRadius(dp(RADIUS_DP));
        shape.setColor(Color.parseColor(BUTTON_COLOR));
        button.setBackground(shape);
        // A button takes focus from a D-pad or a keyboard by default, and the arrow keys then move
        // between buttons instead of reaching the machine's keypad.
        button.setFocusable(false);
        return button;
    }

    private static LinearLayout.LayoutParams item(int top) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.topMargin = top;
        return params;
    }

    private int dp(int value) {
        return (int) TypedValue.applyDimension(
                TypedValue.COMPLEX_UNIT_DIP, value, activity.getResources().getDisplayMetrics());
    }
}
