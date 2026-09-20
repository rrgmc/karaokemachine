package com.rrgmc.karaokemachine;

import android.content.Context;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

/**
 * Who is heard, when more than one application wants to be.
 *
 * <p>Android arbitrates the sound, and an application that does not join in is both rude and
 * unprotected: it talks over whatever was already playing, and a phone call talks over it. This is
 * the Java half of joining in. The Rust side calls {@link MainActivity#requestAudioFocus()} and
 * {@link MainActivity#abandonAudioFocus()}, and hears about changes through
 * {@link #onAudioFocusChange(int)}.
 *
 * <p><b>The listener runs on the main thread and calls straight into Rust.</b> That native stores
 * one integer and returns, because this is the thread an ANR is measured on. The machine's watchdog
 * reads it within fifty milliseconds and does the pausing.
 *
 * <p><b>Ducking is the system's.</b> {@code setWillPauseWhenDucked(false)} tells Android to lower
 * the volume itself for something short, a notification chime being the ordinary case, rather than
 * asking for a pause. A singer sings through a chime; stopping the song for one would be worse than
 * the chime.
 */
final class AudioFocus {

    private static final String TAG = "KaraokeMachine";

    private final AudioManager manager;
    private final AudioFocusRequest request;

    /**
     * Whether the request is outstanding.
     *
     * <p>Guarded by the instance, because the Rust side calls in from its watchdog thread while the
     * listener arrives on the main one. Both are short and neither waits on anything.
     */
    private boolean held;

    AudioFocus(Context context) {
        manager = (AudioManager) context.getSystemService(Context.AUDIO_SERVICE);
        AudioAttributes attributes =
                new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build();
        // The listener is given the main looper outright rather than left to default, so that the
        // thread it arrives on is a decision rather than a discovery.
        request =
                new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                        .setAudioAttributes(attributes)
                        .setWillPauseWhenDucked(false)
                        .setOnAudioFocusChangeListener(
                                AudioFocus::onAudioFocusChange, new Handler(Looper.getMainLooper()))
                        .build();
    }

    /**
     * Asks for the sound, and says whether it was given.
     *
     * <p>A second ask while the request is outstanding is not one: Android would grant it again and
     * the machine would learn nothing, so it answers from what it already holds.
     */
    synchronized boolean request() {
        if (held) {
            return true;
        }
        if (manager == null) {
            Log.w(TAG, "no audio service, so the sound cannot be asked for");
            return false;
        }
        int answer = manager.requestAudioFocus(request);
        held = answer == AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
        if (!held) {
            Log.i(TAG, "audio focus was refused: " + answer);
        }
        return held;
    }

    /** Gives the sound back, if it was ever taken. */
    synchronized void abandon() {
        if (!held || manager == null) {
            return;
        }
        manager.abandonAudioFocusRequest(request);
        held = false;
    }

    /**
     * What Android calls when the sound changes hands.
     *
     * <p>Static, because it hands straight to the native and the native keeps the state. Nothing
     * here may block: see the class comment.
     */
    private static native void onAudioFocusChange(int change);
}
