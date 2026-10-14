package com.rrgmc.karaokemachine;

import android.content.Context;

import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

/**
 * Where the machine in this process answers, as a WebView on this device reaches it.
 *
 * <p>Both shells show the machine's own remote: the headset beside the screen, and the phone over
 * it. Neither calls into Rust for the address. Each reads the {@code settings.json} the machine
 * writes, in the internal storage directory SDL reports to the machine. Java rather than Kotlin,
 * because the phone and television APK compiles no Kotlin.
 */
final class Loopback {

    /** The file and keys {@code crates/machine/karaokemachine/src/settings.rs} writes. */
    private static final String SETTINGS_FILE = "settings.json";
    private static final String API = "api";
    private static final String BIND = "bind";
    private static final String SERVE_REMOTE = "serve_remote";

    /** {@code km_api::DEFAULT_PORT}, which the machine binds when the file names none. */
    private static final int DEFAULT_PORT = 8177;

    private Loopback() {}

    /**
     * The API's root on this device, such as {@code http://127.0.0.1:8177/}.
     *
     * <p>A missing or unreadable file means the default port. The machine writes the file on its
     * first start, and that can come after this.
     */
    static String root(Context context) {
        int port = DEFAULT_PORT;
        JSONObject api = api(context);
        if (api != null) {
            String bind = api.optString(BIND, "");
            try {
                port = Integer.parseInt(bind.substring(bind.lastIndexOf(':') + 1));
            } catch (NumberFormatException ignored) {
                // The default port, as for a file that names none.
            }
        }
        return "http://127.0.0.1:" + port + "/";
    }

    /** Whether the owner serves the singer's remote at {@code /}. It is on unless turned off. */
    static boolean servesRemote(Context context) {
        JSONObject api = api(context);
        return api == null || !Boolean.FALSE.equals(api.opt(SERVE_REMOTE));
    }

    private static JSONObject api(Context context) {
        File file = new File(context.getFilesDir(), SETTINGS_FILE);
        if (!file.isFile()) {
            return null;
        }
        try (InputStream in = new FileInputStream(file)) {
            byte[] bytes = new byte[(int) file.length()];
            int read = 0;
            while (read < bytes.length) {
                int n = in.read(bytes, read, bytes.length - read);
                if (n < 0) {
                    break;
                }
                read += n;
            }
            String text = new String(bytes, 0, read, StandardCharsets.UTF_8);
            return new JSONObject(text).optJSONObject(API);
        } catch (IOException | JSONException e) {
            return null;
        }
    }
}
