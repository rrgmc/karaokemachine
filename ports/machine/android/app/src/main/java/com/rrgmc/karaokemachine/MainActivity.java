package com.rrgmc.karaokemachine;

import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.OpenableColumns;
import android.util.Log;

import org.libsdl.app.SDLActivity;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * The activity Android launches.
 *
 * <p>Everything the machine does is Rust; this exists to tell SDL which libraries to load and to
 * turn a document somebody opened into a file the machine can read. {@link SDLActivity} does the
 * rest — it loads them, sets up JNI, and calls {@code SDL_main}, which {@code km-app}'s library
 * exports.
 */
public class MainActivity extends SDLActivity {

    private static final String TAG = "KaraokeMachine";

    /** What a package must be called to be one. Matched case-insensitively, as the machine does. */
    private static final String PACKAGE_EXTENSION = ".kmpkg";

    /**
     * Where an opened document is copied to, under the app's external files directory.
     *
     * <p>A sibling of {@code packages/} rather than a child of it, so a half-written copy is never
     * in a folder the machine scans. External rather than the cache directory for two reasons: it
     * is the same volume {@code packages/} is on, so the copy the Rust side makes afterwards does
     * not cross one, and the system reclaims a cache directory under pressure, which during a
     * multi-gigabyte copy is the worst moment for it.
     */
    private static final String STAGING_DIR = "incoming";

    /** The name a copy carries until it is whole, which is the rule the Rust side already follows. */
    private static final String PART_SUFFIX = ".part";

    /**
     * The native libraries to load, in dependency order.
     *
     * <p>Order matters and the defaults are wrong for us. {@code SDLActivity} would load
     * {@code SDL3} then {@code main}; our Rust library is {@code km_app}, because naming it
     * {@code karaokemachine} would collide with the desktop binary's output name and naming it
     * {@code main} says nothing.
     *
     * <p>{@code SDL3} comes first because {@code libkm_app.so} links against it, and
     * {@code SDL3_ttf} before it for the same reason. SDL registers its own JNI methods from
     * {@code JNI_OnLoad} when it loads, which is why the {@code Java_org_libsdl_app_*} symbols do
     * not need to be exported from anything.
     */
    @Override
    protected String[] getLibraries() {
        return new String[] {
            "SDL3",
            "SDL3_ttf",
            "km_app",
        };
    }

    /**
     * Takes the launch intent away from SDL before handing over, and handles it here instead.
     *
     * <p>This looks perverse and is not. {@code SDLActivity.onCreate} reads
     * {@code getIntent().getData().getPath()} and hands the result to {@code onNativeDropFile} by
     * itself. That is right for a {@code file:} URI and wrong for the {@code content:} URI every
     * file manager on this platform sends: the path of one is an opaque provider id such as
     * {@code /document/primary:Download/vol1.kmpkg}, which names nothing on disk, so the machine
     * would answer a successful open with a refusal about a file that is not there. Replacing the
     * intent with a bare {@code MAIN} is what stops that, and it keeps the vendored SDL Java stock,
     * which is what the next SDL upgrade wants.
     */
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        Intent opened = getIntent();
        setIntent(new Intent(Intent.ACTION_MAIN));
        super.onCreate(savedInstanceState);
        open(opened);
    }

    /**
     * A second package opened while the machine is already running.
     *
     * <p>The activity is {@code singleInstance}, so this is where that arrives rather than at a new
     * {@code onCreate}, and SDL overrides nothing here — without this the machine would answer the
     * second document by coming to the front and doing nothing.
     */
    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        open(intent);
    }

    /** Stages the document an intent names, if it names one, on a thread of its own. */
    private void open(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction())) {
            return;
        }
        final Uri uri = intent.getData();
        if (uri == null) {
            return;
        }
        // Copying a package is seconds at best and minutes at worst, and the main thread is the one
        // drawing the machine.
        new Thread(() -> stage(uri), "km-stage-package").start();
    }

    /**
     * Copies an opened document somewhere the machine can read, then hands it over as a drop.
     *
     * <p>A drop is the route every platform's documents already take, so nothing in the Rust side
     * is specific to this one: it copies the file into the packages folder under the name its
     * manifest implies, installs it, and says so on the screen.
     */
    private void stage(Uri uri) {
        File dir = getExternalFilesDir(STAGING_DIR);
        if (dir == null) {
            Log.e(TAG, "no external files directory; a package cannot be staged");
            return;
        }
        sweep(dir);

        String name = displayName(uri);
        File staged = new File(dir, name);
        // Refused on its name, before a byte is read, which is what the machine does with a dropped
        // file that is not a package anywhere else. The filter above is broad enough to be offered
        // any unknown binary, so this is the ordinary case and not the odd one, and copying a
        // gigabyte to find out is the thing worth not doing. The path handed over does not exist,
        // and needs not: the refusal is on the extension and never opens it.
        if (!name.toLowerCase().endsWith(PACKAGE_EXTENSION)) {
            SDLActivity.onNativeDropFile(staged.getAbsolutePath());
            return;
        }

        File part = new File(dir, name + PART_SUFFIX);
        try (InputStream in = getContentResolver().openInputStream(uri);
                OutputStream out = new FileOutputStream(part)) {
            if (in == null) {
                Log.e(TAG, "the provider gave nothing to read for " + uri);
                return;
            }
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = in.read(buffer)) > 0) {
                out.write(buffer, 0, read);
            }
        } catch (IOException | SecurityException | IllegalArgumentException error) {
            Log.e(TAG, "a package could not be read out of " + uri, error);
            part.delete();
            return;
        }
        if (!part.renameTo(staged)) {
            Log.e(TAG, "a staged package could not be named " + staged);
            part.delete();
            return;
        }
        SDLActivity.onNativeDropFile(staged.getAbsolutePath());
    }

    /**
     * Empties the staging folder.
     *
     * <p>The side that makes this copy is the side that removes it: the machine never deletes a
     * dropped file, because a file somebody dragged belongs to them, and a copy made here belongs
     * to nobody. Run before each stage rather than after, so a file left behind by an install that
     * did not finish goes at the next one instead of sitting on the volume until the app is
     * uninstalled.
     */
    private void sweep(File dir) {
        File[] leftovers = dir.listFiles();
        if (leftovers == null) {
            return;
        }
        for (File leftover : leftovers) {
            if (leftover.isFile() && !leftover.delete()) {
                Log.w(TAG, "a staged file could not be removed: " + leftover);
            }
        }
    }

    /**
     * What the document is called, as its provider says.
     *
     * <p>The name matters twice: the machine refuses anything not ending in {@code .kmpkg} before
     * reading it, and the band it draws names the file. A provider that answers nothing leaves the
     * URI's last segment, which for a {@code content:} URI is usually an id and will be refused —
     * correctly, since a document with no name is not a package anybody chose.
     *
     * <p>Separators are taken out rather than escaped. A display name is a string a provider hands
     * over, and one carrying a {@code /} would put the copy somewhere this method did not choose.
     */
    private String displayName(Uri uri) {
        String name = null;
        try (Cursor cursor = getContentResolver().query(uri, null, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (column >= 0 && !cursor.isNull(column)) {
                    name = cursor.getString(column);
                }
            }
        } catch (RuntimeException error) {
            Log.w(TAG, "the provider would not say what " + uri + " is called", error);
        }
        if (name == null) {
            name = uri.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            return "package";
        }
        return name.replace('/', '_').replace('\\', '_');
    }
}
