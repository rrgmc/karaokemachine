/*
 * The machine, as an iOS application links it.
 *
 * Hand-written and committed, for three declarations kept as few as the shell can work with.
 * `tests/header_matches.rs` is what keeps this file and `src/ffi.rs` agreeing: a C symbol has
 * no mangling, so a changed signature still links and would go wrong only on a device.
 *
 * Included by `ports/machine/ios/KaraokeMachine/main.m` and `KMPhoneShell.m`, which are the shell.
 *
 * THREADING
 *   Call km_machine_configure on the main thread before SDL_main, and call SDL_main on the thread
 *   SDL is to own. SDL_RunApp arranges the second; `main.m` is where that happens.
 *   km_machine_open_package may be called on any thread while the machine runs.
 *
 * OWNERSHIP
 *   Every string passed in is copied before the call returns, so a caller may free it immediately.
 *   Nothing here returns a pointer, so there is nothing to free.
 */

#ifndef KM_MACHINE_H
#define KM_MACHINE_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Records the two directories the application container gives us.
 *
 *   support    Library/Application Support/karaokemachine -- settings, the catalog, the wallpapers.
 *   documents  Documents -- the folder UIFileSharingEnabled exposes. Packages arrive in
 *              <documents>/packages, which this library appends itself.
 *
 * Create both before calling: this library creates neither. Either may be NULL, which is not fatal
 * and is logged -- the machine then looks where a desktop would, which is a path outside the
 * container that fails visibly on the first write.
 *
 * Calling it twice keeps the first pair and logs the second.
 */
void km_machine_configure(const char *support, const char *documents);

/*
 * Installs a package the person chose in the shell's file browser.
 *
 *   path  the file's path. The machine copies the package into its packages folder, as it does a
 *         dropped file, so the file may be a temporary copy.
 *
 * Queued, and taken by the display loop within a frame. A file that is not a package is refused on
 * the screen. NULL is logged and ignored.
 */
void km_machine_open_package(const char *path);

/*
 * Runs the machine. Does not return until it stops.
 *
 * SDL's own entry point, with SDL's signature. The arguments are ignored: there is no command line
 * on a phone, so every setting comes from settings.json in the directory named above. Returns 0 when
 * the machine stopped normally and 1 when it stopped with an error, neither of which anybody reads
 * -- on a device a return of any kind ends the application.
 */
int SDL_main(int argc, char *argv[]);

#ifdef __cplusplus
}
#endif

#endif /* KM_MACHINE_H */
