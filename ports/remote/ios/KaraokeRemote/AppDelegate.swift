import UIKit

/// The application, which is a window and a view controller.
///
/// Built in code rather than from a storyboard, matching the Android application, which has no
/// layout XML either. No `UISceneDelegate` and no scene manifest: this app has exactly one window
/// and nothing to say about several.
///
/// # What is deliberately not here
///
/// **No `beginBackgroundTask`, no suspend and no resume.** The owner's Go remote needs all three,
/// because its karaoke unit serves five clients and a session abandoned by a suspension holds one
/// of those slots until the unit times it out — so that app holds the session through the grace
/// period the system gives it and gives it up cleanly when that runs out.
///
/// Nothing here is being held. The machine serves any number of callers, this remote never names
/// itself to one, and its link to it is an ordinary HTTP client plus an event stream that
/// reconnects with backoff. The server is *in this process*, so a suspension freezes it along with
/// the app and thaws it again with the port unchanged — which is the thing a forked-child design
/// cannot do, and the reason returning to this app does not reload the page.
///
/// The page survives too, and needs no help: `km-remote-pages`'s `static/live.js` uses a plain
/// `EventSource`, whose reconnection is the browser's own.
@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        let window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = RootViewController()
        // The page's own background, so the moment before the first paint is not a dark flash
        // handing over to a white document.
        window.backgroundColor = .white
        window.makeKeyAndVisible()
        self.window = window
        return true
    }

    /// Best effort only: the system allows a few seconds here and does not promise to call this at
    /// all, so the server is *asked* to stop rather than waited on. `km_remote_stop` returns at
    /// once by design, which is what makes that safe from the main thread.
    func applicationWillTerminate(_ application: UIApplication) {
        Server.stop()
    }
}
