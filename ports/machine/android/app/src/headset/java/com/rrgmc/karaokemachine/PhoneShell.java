package com.rrgmc.karaokemachine;

/**
 * The headset's twin of the phone's controls, and it adds nothing.
 *
 * <p>The queue panel beside the screen already shows the machine's own remote, and the window mode
 * carries its own button. See the {@code flat} flavour's class of this name for what a phone gets.
 */
final class PhoneShell {

    private PhoneShell() {}

    static PhoneShell attach(MainActivity activity) {
        return null;
    }

    boolean back() {
        return false;
    }
}
