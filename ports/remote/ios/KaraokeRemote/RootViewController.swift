import UIKit
import WebKit

/// The whole interface: four screens, of which one is the pages and three are a sentence each.
///
/// *Starting*, the `WKWebView`, *no machine found*, and *failure*. The same four the Android
/// application has, for the same reasons, and the third of them is the one worth knowing about —
/// see `showNoMachine`.
final class RootViewController: UIViewController {
    private var webView: WKWebView?
    /// Polls for a machine while the "no machine found" screen is up. See `watchForMachine`.
    private var watching: DispatchWorkItem?
    /// Where the backup being saved was written.
    ///
    /// Held because `WKDownload` does not report its destination back: the one chosen in
    /// `decideDestinationUsing` is the only place it is known, and `downloadDidFinish` needs it to
    /// offer the file to a share sheet.
    private var downloaded: URL?
    private let addressField = UITextField()

    private static let ink = UIColor(red: 0.106, green: 0.122, blue: 0.137, alpha: 1)
    private static let muted = UIColor(red: 0.416, green: 0.451, blue: 0.490, alpha: 1)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .white
        showStarting()
        Server.start()
        waitForServer()
    }

    /// Comes back to the pages if the failure screen is up and the server has since recovered.
    ///
    /// The Android application does the same in `onResume`, and for the same reason: a screen that
    /// can only be left by pressing a button is a screen somebody can be stuck on.
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if webView == nil && Server.failure == nil && Server.port != nil {
            waitForServer()
        }
    }

    private func waitForServer() {
        Server.waitUntilReady(
            ready: { [weak self] _ in self?.serverIsReady() },
            failed: { [weak self] reason in self?.showFailure(reason) }
        )
    }

    /// One line, and it is why `km_remote_machine` and `km_remote_songs` are in the C surface.
    ///
    /// A machine that was found, or a catalog already on this device, is enough to show the
    /// pages. Neither means a first run on a network where nothing answered, and that is the one
    /// moment somebody has to be offered a box to type an address into.
    private func serverIsReady() {
        if Server.machine == nil && (Server.songs ?? 0) <= 0 {
            showNoMachine()
        } else {
            showWeb()
        }
    }

    // MARK: - The pages

    private func showWeb() {
        // **Keep watching rather than returning into nothing.** The port is published last and only
        // once the server is answering, so this is reachable — and an early return here would stop
        // the watcher without putting anything on screen, which is the dead end the watcher exists
        // to prevent, reintroduced one function along.
        guard let url = Server.url else {
            watchForMachine()
            return
        }
        stopWatching()
        clear()

        let config = WKWebViewConfiguration()
        // The page is htmx plus an event stream, so this is load-bearing rather than a default
        // worth restating.
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        // **Both of these are for the share page's scanner, and neither is redundant with the
        // other.** Without the first, WebKit takes the camera preview fullscreen the moment it
        // plays and the page it belongs to is gone. Without the second, `play()` is blocked —
        // `scan.js` calls it after `getUserMedia` resolves, and by then WebKit no longer counts the
        // tap that started it as a gesture, so the symptom is a frozen first frame rather than an
        // error anybody can read.
        config.allowsInlineMediaPlayback = true
        config.mediaTypesRequiringUserActionForPlayback = []

        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = self
        web.uiDelegate = self
        web.allowsBackForwardNavigationGestures = true
        web.backgroundColor = .white
        web.scrollView.contentInsetAdjustmentBehavior = .never
        web.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(web)

        // **The bottom is pinned to the view and not to the safe area**, and the other three are
        // pinned to the safe area. That asymmetry is the whole of the safe-area handling here:
        // WKWebView reports the insets to the page itself, `km-remote-pages`'s stylesheet already
        // declares `--safe-bottom` with `env(safe-area-inset-bottom)` as its default, and the tab
        // bar is meant to reach the edge. Pin the bottom to the safe area as well and the tab bar
        // floats above a blank strip.
        //
        // The Android application has to measure the insets in Java and push the bottom one into
        // the document as a CSS variable, re-applying it on every navigation, because a plain
        // WebView reports none. Nothing of that is needed here.
        NSLayoutConstraint.activate([
            web.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            web.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor),
            web.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor),
            web.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        webView = web
        web.load(URLRequest(url: url))
    }

    // MARK: - The three screens that are not the pages

    private func showStarting() {
        clear()
        let spinner = UIActivityIndicatorView(style: .medium)
        spinner.startAnimating()
        let label = Self.label(text("starting.title"), color: Self.muted, size: 15)
        center(UIStackView(arrangedSubviews: [spinner, label]), spacing: 12)
    }

    /// The screen a first run on a quiet network lands on.
    ///
    /// **Without the watcher below this is a dead end, and on Android it was one.** The first look
    /// finds nothing, this screen appears, and twenty seconds later the server quietly finds the
    /// machine, copies the song list and connects — with the screen still saying nothing answered.
    private func showNoMachine() {
        clear()
        let title = Self.label(text("no_machine.title"), color: Self.ink, size: 17, weight: .semibold)
        let detail = Self.label(
            text("no_machine.body"),
            color: Self.muted, size: 13
        )

        addressField.placeholder = text("address.placeholder")
        addressField.text = Settings.machineAddress ?? ""
        addressField.borderStyle = .roundedRect
        addressField.keyboardType = .URL
        addressField.autocapitalizationType = .none
        addressField.autocorrectionType = .no

        let retry = Self.button(text("action.retry"), action: #selector(retryTapped))
        let carryOn = Self.button(text("action.carry_on"), action: #selector(carryOnTapped))
        // **Naming a machine pins it**, so the way back out has to be visible: an address that was
        // typed wrongly would otherwise be permanent, and the server would never look again.
        let hint = Self.label(
            text("no_machine.note"),
            color: Self.muted, size: 12
        )

        center(
            UIStackView(arrangedSubviews: [title, detail, addressField, retry, carryOn, hint]),
            spacing: 14
        )
        watchForMachine()
    }

    private func showFailure(_ reason: String) {
        stopWatching()
        clear()
        let title = Self.label(text("failed.title"), color: Self.ink, size: 17, weight: .semibold)
        let detail = Self.label(reason, color: Self.muted, size: 13)
        let retry = Self.button(text("action.try_again"), action: #selector(retryTapped))
        center(UIStackView(arrangedSubviews: [title, detail, retry]), spacing: 14)
    }

    /// The server is fine and the page would not load — a different thing from the screen above, and
    /// worth different words. *Reload* asks the web view again rather than restarting a server that
    /// has nothing wrong with it.
    private func showPageFailure(_ reason: String) {
        stopWatching()
        clear()
        let title = Self.label(text("page_failed.title"), color: Self.ink, size: 17, weight: .semibold)
        let detail = Self.label(reason, color: Self.muted, size: 13)
        let reload = Self.button("Reload", action: #selector(reloadTapped))
        center(UIStackView(arrangedSubviews: [title, detail, reload]), spacing: 14)
    }

    @objc private func reloadTapped() {
        showWeb()
    }

    /// Replaces the "no machine found" screen if one turns up while somebody is reading it.
    ///
    /// Half a second, and only while that screen is up. The sweep is behind the recovery loop as
    /// well as in front of the pages, so a machine switched on after the app opened is found within
    /// twenty seconds without anybody doing anything.
    private func watchForMachine() {
        stopWatching()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.webView == nil else { return }
            if Server.machine != nil {
                self.showWeb()
            } else {
                self.watchForMachine()
            }
        }
        watching = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
    }

    private func stopWatching() {
        watching?.cancel()
        watching = nil
    }

    @objc private func retryTapped() {
        // Saved even when empty, which is how a pinned machine is given up.
        Settings.machineAddress = addressField.text
        stopWatching()
        showStarting()
        // Off the main thread only because `stop` and `start` are a pair here: `stop` returns at
        // once, but doing both on the main queue would still run the whole handover inside a touch
        // event for no reason.
        DispatchQueue.global(qos: .userInitiated).async {
            Server.stop()
            Server.start()
            DispatchQueue.main.async { [weak self] in self?.waitForServer() }
        }
    }

    /// Shows the pages with an empty catalog and no machine, which is a legitimate thing to want:
    /// the favorites are on this device and the song list will fill in when a machine appears.
    @objc private func carryOnTapped() {
        showWeb()
    }

    // MARK: - Layout helpers

    private func clear() {
        view.subviews.forEach { $0.removeFromSuperview() }
        // `removeFromSuperview` alone leaves the renderer and the event stream open on it, so the
        // page would go on receiving events it can no longer show. The Android application records
        // the same rule about `WebView.destroy`.
        webView?.navigationDelegate = nil
        webView?.uiDelegate = nil
        webView = nil
    }

    private func center(_ stack: UIStackView, spacing: CGFloat) {
        stack.axis = .vertical
        stack.alignment = .fill
        stack.spacing = spacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 28),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -28),
        ])
    }

    /// One of the shell's own strings, in whatever language the device asked for.
    ///
    /// **Apple's resources rather than this project's `.ftl` catalogs**, for the reason
    /// `en.lproj/Localizable.strings` gives: the *page* is `km-remote-pages` and speaks Fluent
    /// because it travels to five hosts, and everything drawn *before* the page is Swift. A second
    /// locale mechanism inside one app is one too many, and this is the one the platform already
    /// resolves against the device's preferred languages — the same question `Accept-Language`
    /// answers a moment later, so the two agree without being told to.
    ///
    /// A free function rather than `NSLocalizedString` at each site, so the table name is written
    /// once and a key is what a reader sees.
    private func text(_ key: String) -> String {
        NSLocalizedString(key, comment: "")
    }

    private static func label(
        _ text: String, color: UIColor, size: CGFloat, weight: UIFont.Weight = .regular
    ) -> UILabel {
        let label = UILabel()
        label.text = text
        label.textColor = color
        label.font = .systemFont(ofSize: size, weight: weight)
        label.numberOfLines = 0
        label.textAlignment = .center
        return label
    }

    private func button(_ title: String, action: Selector) -> UIButton {
        Self.button(title, action: action)
    }

    private static func button(_ title: String, action: Selector) -> UIButton {
        let button = UIButton(type: .system)
        button.setTitle(title, for: .normal)
        button.titleLabel?.font = .systemFont(ofSize: 16, weight: .medium)
        button.addTarget(nil, action: action, for: .touchUpInside)
        return button
    }
}

extension RootViewController: WKNavigationDelegate {
    /// Decided by host, never by naming a site.
    ///
    /// Anything on loopback stays inside; everything else is handed to whatever app claims it, and
    /// to the browser if none does. A nil host — `about:blank`, a `data:` URL — counts as ours.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
        guard let url = navigationAction.request.url else {
            decisionHandler(.allow)
            return
        }
        if isOurs(url) {
            decisionHandler(.allow)
            return
        }
        // Only a deliberate act leaves the app. Anything else on a foreign host is refused rather
        // than opened, so a stray subresource cannot send somebody to Safari.
        if navigationAction.navigationType == .linkActivated {
            UIApplication.shared.open(url)
        }
        decisionHandler(.cancel)
    }

    private func isOurs(_ url: URL) -> Bool {
        isOurHost(url.host)
    }

    /// Whether a host is this app's own server.
    ///
    /// **Split out of `isOurs` so the navigation rule and the camera rule are one rule.** The
    /// permission delegate is handed a `WKSecurityOrigin`, whose host is a `String` rather than part
    /// of a `URL`, and a second copy of this test is how the two quietly come to disagree. One
    /// place is what lets that delegate's comment say "this view only ever loads our own server"
    /// and have it hold by construction.
    ///
    /// A relative URL has no host and is ours by definition. `Server` builds
    /// `http://127.0.0.1:<port>/` literally, so the `localhost` arm is defensive rather than a path
    /// anything takes.
    private func isOurHost(_ host: String?) -> Bool {
        guard let host else { return true }
        return host == "127.0.0.1" || host == "localhost"
    }

    /// Turns the backup route's response into a file rather than a page.
    ///
    /// **Decided by the header, not by naming the URL**, which is the rule the rest of this file
    /// follows: the server says what is a file, so anything it later serves as an attachment is
    /// handled here without this having to learn about it.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationResponse: WKNavigationResponse,
        decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
    ) {
        guard let response = navigationResponse.response as? HTTPURLResponse,
            let disposition = response.value(forHTTPHeaderField: "Content-Disposition"),
            disposition.lowercased().hasPrefix("attachment")
        else {
            decisionHandler(.allow)
            return
        }
        decisionHandler(.download)
    }

    func webView(
        _ webView: WKWebView,
        navigationResponse: WKNavigationResponse,
        didBecome download: WKDownload
    ) {
        download.delegate = self
    }

    /// The server is in this process, so a page that will not load means the server has stopped
    /// serving — not that the address is wrong.
    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
    ) {
        showLoadFailure(error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        showLoadFailure(error)
    }

    /// The renderer died, which the page cannot recover from by itself.
    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        showWeb()
    }

    /// **Not every navigation error is a failure, and treating them alike cost a session.**
    ///
    /// Two arrive in ordinary use and mean nothing is wrong. A load that is superseded — which is
    /// exactly what `showWeb` does when the watcher finds a machine while a page is already loading
    /// — reports `NSURLErrorCancelled`. And a `.cancel` decision, which is what every link to a
    /// foreign host gets, can surface as `WebKitErrorDomain` 102, "frame load interrupted by a
    /// policy change".
    ///
    /// Reading either as the server having died is what put "The remote did not start" on screen at
    /// the exact moment the machine had just been found, with a healthy server behind it and a
    /// *Try again* button that "fixed" it by restarting something that was never broken. The owner's
    /// Go remote records the same trap and filters the same code.
    ///
    /// Matched by domain and number because `WebKitErrorFrameLoadInterruptedByPolicyChange` is not
    /// exposed to Swift, and filtered in **both** failure callbacks because which one it arrives at
    /// has varied between WebKit versions.
    ///
    /// **A third case joined the two above, and it is the routine one now.** Turning a navigation
    /// into a download *cancels that navigation*, so every backup saved from the share pages arrives
    /// here as a 102 — where before it was the rare consequence of a foreign link being refused. Do
    /// not narrow this to "only cancellations on a foreign host": that reading would put a failure
    /// screen up every time somebody saved their favorites, and take the page they were on away with
    /// it. The Go remote this follows hit exactly that, and worse, because its failure path
    /// *restarted the server* — so saving a backup tore down the server that had just served it.
    private func isHarmless(_ error: Error) -> Bool {
        let error = error as NSError
        if error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled { return true }
        if error.domain == "WebKitErrorDomain" && error.code == 102 { return true }
        return false
    }

    private func showLoadFailure(_ error: Error) {
        guard !isHarmless(error) else { return }
        // **The server's own reason first, and the two are different questions.** If the server has
        // stopped, that is what somebody needs to read; if it is healthy and only the page would not
        // load, saying "the remote did not start" is a lie that sends them to the wrong fix.
        if let failure = Server.failure {
            showFailure(failure)
        } else {
            showPageFailure(error.localizedDescription)
        }
    }
}

/// Answers the page's `alert()` and `confirm()`.
///
/// **Without this the page loses controls, silently.** A `WKWebView` with no `WKUIDelegate` does not
/// merely skip the dialog: `runJavaScriptConfirmPanel` has no default, so `confirm()` returns
/// `false` with nothing on screen. htmx reads that as "the person said no" and never sends the
/// request, which is what took ✕ off the Queue tab — the button was pressed, the queue did not
/// change, and there was nothing to say why. The ↑ and ↓ beside it worked, because they carry no
/// `hx-confirm`.
///
/// The Android application had the same hole for the same reason, one framework over: a `WebView`
/// with no `WebChromeClient` cancels the request outright.
///
/// **The completion handler must be called exactly once.** WebKit traps a second call, and a page
/// whose handler is never called stays frozen with its script suspended — so every way out of the
/// alert has to end in one, which is why neither of the two dialogs is given a cancel-on-dismiss: a
/// `UIAlertController` presented this way has no way out but its own buttons.
///
/// **That rule now covers three handlers**, and the third is the one where it bites hardest. A
/// `decisionHandler` that is never called leaves `getUserMedia` pending for ever rather than
/// failing, so the share page's scanner would sit on "Starting the camera…" with nothing to say —
/// which is the identical failure the Android shell's `PermissionRequest` has, in the identical
/// shape, one framework over.
extension RootViewController: WKUIDelegate {
    func webView(
        _ webView: WKWebView,
        runJavaScriptAlertPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping () -> Void
    ) {
        let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler() })
        present(alert, animated: true)
    }

    func webView(
        _ webView: WKWebView,
        runJavaScriptConfirmPanelWithMessage message: String,
        initiatedByFrame frame: WKFrameInfo,
        completionHandler: @escaping (Bool) -> Void
    ) {
        let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
        // Cancel leads, which is the order iOS uses for a question whose yes takes something away.
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in completionHandler(false) })
        alert.addAction(UIAlertAction(title: "OK", style: .default) { _ in completionHandler(true) })
        present(alert, animated: true)
    }

    /// Grants the camera to the share page's scanner.
    ///
    /// **Not redundant with `NSCameraUsageDescription`, and the two answer different questions.**
    /// WebKit's own permission prompt is *never remembered* in a `WKWebView` — it is raised on every
    /// `getUserMedia` call, so a second press of Scan asked a second time and there was no way to
    /// answer once. Implementing this replaces that prompt, which leaves iOS's app-level camera
    /// prompt as the only one anybody sees: asked once, remembered, and revocable in Settings. The
    /// plist key is what makes that prompt legal to raise at all; without it the app is *killed*
    /// rather than refused.
    ///
    /// **It is also what makes the receive page's auto-start viable** rather than a tidy-up:
    /// `scan.js` calls `start()` on load with no tap, and without this WebKit's unremembered prompt
    /// would appear on every visit instead of on every button press. Granting here removes the
    /// gesture requirement with it, because that rule is tied to putting the prompt up.
    ///
    /// Only `.camera`, and only for our own host — checked through ``isOurHost(_:)`` rather than
    /// assumed, so "this view only ever loads our own server" holds by construction. A microphone
    /// request is refused outright: nothing here opens an input stream, and `scan.js` asks for
    /// `audio: false`.
    func webView(
        _ webView: WKWebView,
        requestMediaCapturePermissionFor origin: WKSecurityOrigin,
        initiatedByFrame frame: WKFrameInfo,
        type: WKMediaCaptureType,
        decisionHandler: @escaping (WKPermissionDecision) -> Void
    ) {
        guard type == .camera, isOurHost(origin.host) else {
            NSLog("km-remote: refused a \(type) capture request from \(origin.host)")
            decisionHandler(.deny)
            return
        }
        decisionHandler(.grant)
    }
}

/// Writes a saved backup somewhere, and offers it onward.
///
/// **A fresh directory per download, under `temporaryDirectory`**, and both halves of that are
/// deliberate. WebKit refuses a destination that already exists, so two backups saved on one day —
/// which carry the same dated filename — would collide on the second. And `temporaryDirectory`
/// rather than `Documents` because `Documents` is iCloud-backed *and* exposed in the Files app:
/// `Server.excludeMirrorFromBackup()` names `catalog.sqlite` and its `-wal`/`-shm` siblings in a
/// literal array precisely so the collection stays in iCloud and the derived copy stays out, and a
/// backup written under `Documents` would be a third thing that array had to know about. Written
/// here, it is backed up by nothing and that array stays a two-file array.
///
/// The temporary directory is not cleaned up afterwards, on purpose: some share extensions read the
/// URL after the sheet has dismissed, and the system reclaims `tmp` on its own.
extension RootViewController: WKDownloadDelegate {
    func download(
        _ download: WKDownload,
        decideDestinationUsing response: URLResponse,
        suggestedFilename: String,
        completionHandler: @escaping (URL?) -> Void
    ) {
        let name = suggestedFilename.isEmpty ? "km-favorites.json" : suggestedFilename
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("favorites-\(UUID().uuidString)", isDirectory: true)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        } catch {
            NSLog("km-remote: could not make a place for the backup: \(error)")
            completionHandler(nil)
            return
        }
        let file = folder.appendingPathComponent(name)
        downloaded = file
        completionHandler(file)
    }

    func downloadDidFinish(_ download: WKDownload) {
        guard let file = downloaded else { return }
        downloaded = nil
        share(file)
    }

    /// **Its own alert, and emphatically not `showLoadFailure`.**
    ///
    /// A backup that could not be written is a thing that did not happen; the server is fine and the
    /// page behind this is fine. `showLoadFailure` leads to `showFailure`/`showPageFailure`, both of
    /// which call `clear()` — so routing this there would take the page off screen because a file
    /// did not get saved.
    func download(_ download: WKDownload, didFailWithError error: Error, resumeData: Data?) {
        NSLog("km-remote: the backup was not saved: \(error)")
        downloaded = nil
        let alert = UIAlertController(
            title: text("save_failed.title"),
            message: nil,
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
    }

    /// Hands the file to Files, Dropbox, Drive or whatever else is installed.
    ///
    /// **The popover anchor is required rather than tidy.** On an iPad a share sheet with no anchor
    /// *raises* instead of presenting, and an iPad is the device this port has actually been run on
    /// — see `ports/remote/ios/README.md`. Anchored to the middle of the view, because the control
    /// that started this is inside the web view and has no frame this side of it.
    private func share(_ file: URL) {
        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        if let popover = sheet.popoverPresentationController {
            popover.sourceView = view
            popover.sourceRect = CGRect(x: view.bounds.midX, y: view.bounds.midY, width: 0, height: 0)
            popover.permittedArrowDirections = []
        }
        present(sheet, animated: true)
    }
}
