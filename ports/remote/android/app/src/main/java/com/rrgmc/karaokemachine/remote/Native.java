package com.rrgmc.karaokemachine.remote;

/**
 * The server, running inside this process.
 *
 * <p>There is no child process here and no output to scrape. The Go remote this application follows
 * ships its server as a file named {@code lib*.so}, extracts it, {@code exec}s it and reads the
 * port out of the child's stdout — but that is a Go constraint rather than an Android one, because a
 * cgo-free Go binary cannot be loaded as a library. This is a real shared library: {@code
 * System.loadLibrary} maps it, {@link #start} spawns the server on a Tokio runtime in this process,
 * and {@link #port} reads the bound port straight back across the boundary.
 *
 * <p>Every method is a static native with a mangled name, so the linker checks them rather than a
 * runtime signature table. All of them are safe to call from any thread and none of them blocks:
 * {@code start} returns as soon as the server has been spawned, and {@code stop} returns as soon as
 * it has been asked to stop, which is what keeps both callable from the main looper.
 */
final class Native {

    /** The tag every line from the Rust side carries. {@code adb logcat -s km-remote}. */
    static final String TAG = "km-remote";

    static {
        System.loadLibrary("km_remote_android");
    }

    private Native() {
    }

    /**
     * Starts the server, and returns at once.
     *
     * <p>A no-op if one is already running, which is what makes it safe to call from an Activity
     * that may be recreated without checking first.
     *
     * @param dataDir where the catalog mirror and the favorites live. There is no default and
     *     there must not be one: working a data directory out from the location of the executable is
     *     a desktop idea, and on a phone the caller is the only thing that knows the answer.
     * @param machine an address somebody typed, or {@code null} to try what was remembered and then
     *     the network. <b>Naming one pins it</b> — the server never wanders away from a machine it
     *     was told about, however long that machine stays silent — so anything offering this must
     *     also offer a way to clear it.
     */
    static native void start(String dataDir, String machine);

    /** The port the server is answering on, or 0 until it is. */
    static native int port();

    /** Why the server stopped, or {@code null} while it is healthy. */
    static native String failure();

    /**
     * The machine this run is talking to, or {@code null}.
     *
     * <p>{@code null} is ordinary and is <b>not</b> a failure: browsing, searching and favorites
     * all answer from this device's own copy with no machine at all, which is the entire reason the
     * offline remote exists.
     */
    static native String machine();

    /**
     * How many songs this device's copy of the catalog holds, or -1 if that is not known yet.
     *
     * <p>Zero is a real answer and the interesting one: a first run that found no machine and has
     * nothing to show. It is the only signal that tells that case apart from a working app whose
     * user simply has not searched yet.
     */
    static native int songs();

    /** Asks the server to stop. Returns at once. */
    static native void stop();
}
