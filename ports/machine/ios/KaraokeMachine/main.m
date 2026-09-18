/*
 * The machine's entry point on iOS.
 *
 * **Objective-C and a plain `main`, where the offline remote's shell is Swift with `@main` on its
 * app delegate.** The difference is who owns `UIApplicationMain`. That shell draws its own window
 * and puts a WKWebView in it, so Swift is the application. Here SDL is: `SDL_RunApp` calls
 * `UIApplicationMain` with SDL's own delegate, which creates the window, the Metal layer and the
 * event pump the machine draws through. A second `@main` beside it would be two applications
 * competing for one process.
 *
 * So this file is the whole shell, and it does three things before handing over:
 *
 *   1. Names the two directories the container gives us, and creates them.
 *   2. Configures the audio session, which cpal's CoreAudio backend does not do.
 *   3. Calls km_machine_configure, then SDL_RunApp, which does not return.
 *
 * SDL's headers are deliberately not reachable from this target. SDL is compiled into
 * libkm_machine_ios.a and its headers are not copied out of cargo's build directory, so the two
 * functions needed from it are declared here rather than included. Both are public SDL API with
 * signatures fixed by SDL 3.
 */

#import <UIKit/UIKit.h>
#import <AVFoundation/AVFoundation.h>

#import "km_machine.h"

/* SDL's own, declared rather than included -- see the header comment. */
typedef int (*km_sdl_main_func)(int argc, char *argv[]);
extern int SDL_RunApp(int argc, char *argv[], km_sdl_main_func mainFunction, void *reserved);

/*
 * The directory, created if it is not there.
 *
 * Returns nil when it cannot be made, which km_machine_configure treats as "not given": the machine
 * then looks where a desktop would and fails visibly on the first write, rather than starting on a
 * catalog it silently cannot save.
 */
static NSString *km_ensure(NSString *path) {
    NSError *error = nil;
    BOOL made = [[NSFileManager defaultManager] createDirectoryAtPath:path
                                          withIntermediateDirectories:YES
                                                           attributes:nil
                                                                error:&error];
    if (!made) {
        NSLog(@"karaokemachine: could not create %@: %@", path, error);
        return nil;
    }
    return path;
}

/*
 * Application Support, which is not exposed to the person and is backed up.
 *
 * `NSApplicationSupportDirectory` inside the container, plus the application's own name -- the
 * system does not make that subdirectory and an application that skipped it would keep its settings
 * loose beside every other framework's.
 */
static NSString *km_support_dir(void) {
    NSArray<NSString *> *roots = NSSearchPathForDirectoriesInDomains(
        NSApplicationSupportDirectory, NSUserDomainMask, YES);
    if (roots.count == 0) {
        return nil;
    }
    return km_ensure([roots.firstObject stringByAppendingPathComponent:@"karaokemachine"]);
}

/*
 * Documents, which UIFileSharingEnabled exposes in the Files app and in Finder over USB.
 *
 * The `packages` child is appended by the Rust side rather than here, so that one place decides what
 * the container's layout is. It is created here all the same: a folder somebody is meant to drop a
 * `.kmpkg` into has to exist before they look for it.
 */
static NSString *km_documents_dir(void) {
    NSArray<NSString *> *roots = NSSearchPathForDirectoriesInDomains(
        NSDocumentDirectory, NSUserDomainMask, YES);
    if (roots.count == 0) {
        return nil;
    }
    NSString *documents = roots.firstObject;
    km_ensure([documents stringByAppendingPathComponent:@"packages"]);
    return documents;
}

/*
 * Playback, and active.
 *
 * **cpal's CoreAudio backend configures no session at all**, and the one an application gets by
 * default is `soloAmbient`: the ringer switch silences it, and it stops the moment the application
 * leaves the foreground. Both are wrong for a machine somebody is singing along to, and neither
 * reports anything -- the obvious reading of that silence is a broken audio path rather than a
 * session nobody asked for. It is the same shape of fault as the JavaVM cpal expected somebody else
 * to have published on Android, and it is found the same way: by the audio being absent while every
 * log line says the engine started.
 *
 * **The category is the whole of it, and there is no background mode beside it.** The machine pauses
 * when the screen goes away, so it plays nothing there and declares no `UIBackgroundModes`; the
 * system is then free to interrupt this session and suspend the process, which is what keeps a
 * backgrounded machine off the battery. See `The machine sleeps when it leaves the screen` in
 * docs/decisions/audio.md.
 *
 * Before SDL, and before the first output unit is opened. SDL's own audio subsystem is never
 * initialized here -- the machine's audio is cpal's -- so nothing downstream will set this or
 * overwrite it.
 *
 * A failure is logged and not fatal. A machine that cannot configure a session is still one somebody
 * can queue songs on and read a screen from, and refusing to start would trade a quiet fault for a
 * total one.
 */
static void km_configure_audio_session(void) {
    AVAudioSession *session = [AVAudioSession sharedInstance];
    NSError *error = nil;
    if (![session setCategory:AVAudioSessionCategoryPlayback error:&error]) {
        NSLog(@"karaokemachine: could not set the audio category: %@", error);
        return;
    }
    if (![session setActive:YES error:&error]) {
        NSLog(@"karaokemachine: could not activate the audio session: %@", error);
    }
}

int main(int argc, char *argv[]) {
    @autoreleasepool {
        NSString *support = km_support_dir();
        NSString *documents = km_documents_dir();
        km_configure_audio_session();
        /* fileSystemRepresentation, not UTF8String: a container path is what the file system says
         * it is, and this is the encoding the Rust side will open it with. */
        km_machine_configure(support.fileSystemRepresentation,
                             documents.fileSystemRepresentation);
    }
    /* Outside the pool: this does not return, so anything held by it would never be drained. */
    return SDL_RunApp(argc, argv, SDL_main, NULL);
}
