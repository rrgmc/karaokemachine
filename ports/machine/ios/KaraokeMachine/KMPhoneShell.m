/*
 * The controls an iPhone or an iPad adds over the machine: its own remote, and a way to install a
 * package.
 *
 * On a phone the machine and the singer are one device. Switching to the remote application takes
 * the screen away, and iOS then suspends the machine and closes its listening socket. This shows the
 * machine's own remote over the screen instead, in the same window. The application stays in front,
 * so SDL goes on drawing and nothing pauses. The page comes from this process over loopback, so no
 * network stands between the two.
 *
 * Every view here is a plain subview of SDL's own view. A second window would have to take the key
 * status for the page to receive typing.
 *
 * Rust calls km_phone_shell_attach once the renderer exists, because creating the renderer replaces
 * SDL's view with a Metal view. Controls added before that would leave the window with the old view.
 *
 * The Android twin is ports/machine/android/app/src/flat/java/com/rrgmc/karaokemachine/PhoneShell.java,
 * and the decision both follow is `On a phone, the machine shows its own remote over the screen` in
 * docs/decisions/remotes.md.
 */

#import "KMPhoneShell.h"
#import "km_machine.h"

#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

/* How long a page that did not load waits before it tries again. */
static const NSTimeInterval KMRetrySeconds = 2.0;

static const CGFloat KMTextPoints = 14;
static const CGFloat KMPadX = 14;
static const CGFloat KMPadY = 6;
static const CGFloat KMMargin = 8;
static const CGFloat KMGap = 6;

/* The strip beside the open remote that holds Close, so the button covers none of the page. */
static const CGFloat KMRail = 88;

/* The menu button over a song: visible enough to find, faint enough not to cover words. */
static const CGFloat KMIdleAlpha = 0.55;

/* `km_api::DEFAULT_PORT`, which the machine binds when settings.json names none. */
static const NSInteger KMDefaultPort = 8177;

/* The package type project.yml exports. */
static NSString *const KMPackageType = @"com.karaokemachine.kmpkg";

/* Where settings.json is, from main. */
static NSString *km_support;

static UIColor *km_button_color(void) {
    return [UIColor colorWithRed:0x1F / 255.0 green:0x6F / 255.0 blue:0xEB / 255.0 alpha:1];
}

static UIColor *km_page_color(void) {
    return [UIColor colorWithRed:0x10 / 255.0 green:0x14 / 255.0 blue:0x18 / 255.0 alpha:1];
}

/*
 * The `api` object of the settings.json the machine writes, or nil.
 *
 * The file and keys are those `crates/machine/karaokemachine/src/settings.rs` writes. A missing or
 * unreadable file means the defaults: the machine writes the file on its first start, and that can
 * come after this.
 */
static NSDictionary *km_api_settings(void) {
    if (km_support == nil) {
        return nil;
    }
    NSString *path = [km_support stringByAppendingPathComponent:@"settings.json"];
    NSData *data = [NSData dataWithContentsOfFile:path];
    if (data == nil) {
        return nil;
    }
    id root = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (![root isKindOfClass:[NSDictionary class]]) {
        return nil;
    }
    id api = root[@"api"];
    return [api isKindOfClass:[NSDictionary class]] ? api : nil;
}

/* The API's root on this device, such as `http://127.0.0.1:8177/`. */
static NSURL *km_loopback_root(void) {
    NSInteger port = KMDefaultPort;
    id bind = km_api_settings()[@"bind"];
    if ([bind isKindOfClass:[NSString class]]) {
        NSRange colon = [bind rangeOfString:@":" options:NSBackwardsSearch];
        if (colon.location != NSNotFound) {
            NSInteger named = [[bind substringFromIndex:colon.location + 1] integerValue];
            if (named > 0 && named <= 65535) {
                port = named;
            }
        }
    }
    return [NSURL URLWithString:[NSString stringWithFormat:@"http://127.0.0.1:%ld/", (long)port]];
}

/* Whether the owner serves the singer's remote at `/`. It is on unless turned off. */
static BOOL km_serves_remote(void) {
    id serve = km_api_settings()[@"serve_remote"];
    return !([serve isKindOfClass:[NSNumber class]] && ![serve boolValue]);
}

/* Whether a host is this process's own server. A URL with no host, such as `about:blank`, is. */
static BOOL km_is_ours(NSURL *url) {
    NSString *host = url.host;
    return host == nil || [host isEqualToString:@"127.0.0.1"] || [host isEqualToString:@"localhost"];
}

/*
 * A layer that takes no touch itself, so a touch that misses every control in it reaches the
 * machine below.
 */
@interface KMPassThroughView : UIView
@end

@implementation KMPassThroughView
- (UIView *)hitTest:(CGPoint)point withEvent:(UIEvent *)event {
    UIView *hit = [super hitTest:point withEvent:event];
    return hit == self ? nil : hit;
}
@end

@interface KMPhoneShell : NSObject <WKNavigationDelegate, WKUIDelegate, UIDocumentPickerDelegate>
@end

@implementation KMPhoneShell {
    UIViewController *_controller;
    KMPassThroughView *_layer;
    UIStackView *_column;
    UIView *_rail;
    UIButton *_menu;
    UIButton *_songs;
    UIButton *_install;
    /* The remote, built on first use and kept, so the page keeps its place between openings. */
    WKWebView *_page;
    NSURL *_address;
    BOOL _expanded;
}

- (instancetype)initOver:(UIViewController *)controller {
    self = [super init];
    if (self == nil) {
        return nil;
    }
    _controller = controller;
    UIView *host = controller.view;

    _layer = [[KMPassThroughView alloc] initWithFrame:host.bounds];
    _layer.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    [host addSubview:_layer];

    _menu = [self button:@"Menu" action:@selector(toggle)];
    _songs = [self button:@"Choose songs" action:@selector(show)];
    _install = [self button:@"Install a package" action:@selector(choosePackage)];

    // Top centre, because the machine draws in both top corners: the song's title on the left, and
    // the key, the melody and the queue counter on the right.
    _column = [[UIStackView alloc] initWithArrangedSubviews:@[ _menu, _songs, _install ]];
    _column.axis = UILayoutConstraintAxisVertical;
    _column.alignment = UIStackViewAlignmentCenter;
    _column.spacing = KMGap;
    _column.translatesAutoresizingMaskIntoConstraints = NO;
    [_layer addSubview:_column];

    // The open remote's own controls fill its edges, so Close stands beside the page and not on it.
    // The rail reaches the screen's edge, and its button stays inside the safe area.
    _rail = [[UIView alloc] init];
    _rail.backgroundColor = km_page_color();
    _rail.translatesAutoresizingMaskIntoConstraints = NO;
    _rail.hidden = YES;
    UIButton *close = [self button:@"Close" action:@selector(close)];
    [_rail addSubview:close];
    [_layer addSubview:_rail];

    UILayoutGuide *safe = _layer.safeAreaLayoutGuide;
    [NSLayoutConstraint activateConstraints:@[
        [_column.topAnchor constraintEqualToAnchor:safe.topAnchor constant:KMMargin],
        [_column.centerXAnchor constraintEqualToAnchor:safe.centerXAnchor],
        [_rail.topAnchor constraintEqualToAnchor:_layer.topAnchor],
        [_rail.bottomAnchor constraintEqualToAnchor:_layer.bottomAnchor],
        [_rail.trailingAnchor constraintEqualToAnchor:_layer.trailingAnchor],
        [_rail.leadingAnchor constraintEqualToAnchor:safe.trailingAnchor constant:-KMRail],
        [close.topAnchor constraintEqualToAnchor:safe.topAnchor constant:KMMargin],
        [close.centerXAnchor constraintEqualToAnchor:_rail.leadingAnchor constant:KMRail / 2],
    ]];

    [self expand:NO];
    return self;
}

- (UIButton *)button:(NSString *)title action:(SEL)action {
    UIButtonConfiguration *config = [UIButtonConfiguration filledButtonConfiguration];
    config.baseBackgroundColor = km_button_color();
    config.baseForegroundColor = UIColor.whiteColor;
    config.cornerStyle = UIButtonConfigurationCornerStyleCapsule;
    config.contentInsets = NSDirectionalEdgeInsetsMake(KMPadY, KMPadX, KMPadY, KMPadX);
    config.attributedTitle = [[NSAttributedString alloc]
        initWithString:title
            attributes:@{NSFontAttributeName : [UIFont systemFontOfSize:KMTextPoints]}];
    UIButton *button = [UIButton buttonWithConfiguration:config primaryAction:nil];
    [button addTarget:self action:action forControlEvents:UIControlEventTouchUpInside];
    button.translatesAutoresizingMaskIntoConstraints = NO;
    return button;
}

- (void)toggle {
    [self expand:!_expanded];
}

/*
 * Shows or hides the two items under the menu button.
 *
 * The owner can turn the machine's remote off, and then the page is not there to show. The setting
 * is read at each opening, because the remote's own settings page can change it.
 */
- (void)expand:(BOOL)on {
    _expanded = on;
    _songs.hidden = !(on && km_serves_remote());
    _install.hidden = !on;
    _menu.alpha = on ? 1 : KMIdleAlpha;
}

- (void)show {
    [self expand:NO];
    if (_page == nil) {
        _address = km_loopback_root();
        _page = [self page];
        [_layer insertSubview:_page atIndex:0];
        [NSLayoutConstraint activateConstraints:@[
            [_page.topAnchor constraintEqualToAnchor:_layer.topAnchor],
            [_page.bottomAnchor constraintEqualToAnchor:_layer.bottomAnchor],
            [_page.leadingAnchor constraintEqualToAnchor:_layer.leadingAnchor],
            [_page.trailingAnchor constraintEqualToAnchor:_rail.leadingAnchor],
        ]];
        [_page loadRequest:[NSURLRequest requestWithURL:_address]];
    }
    _page.hidden = NO;
    _rail.hidden = NO;
    _column.hidden = YES;
}

- (void)close {
    _page.hidden = YES;
    _rail.hidden = YES;
    _column.hidden = NO;
}

/* The machine's remote. The remote is htmx: it swaps fragments with script. */
- (WKWebView *)page {
    WKWebViewConfiguration *config = [[WKWebViewConfiguration alloc] init];
    config.allowsInlineMediaPlayback = YES;
    WKWebView *web = [[WKWebView alloc] initWithFrame:CGRectZero configuration:config];
    web.translatesAutoresizingMaskIntoConstraints = NO;
    web.opaque = NO;
    web.backgroundColor = km_page_color();
    web.navigationDelegate = self;
    web.UIDelegate = self;
    return web;
}

- (void)retry {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(KMRetrySeconds * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                       [self->_page loadRequest:[NSURLRequest requestWithURL:self->_address]];
                   });
}

/* Retries until the machine answers, which can be after the page is first opened. */
- (void)webView:(WKWebView *)webView
    didFailProvisionalNavigation:(WKNavigation *)navigation
                       withError:(NSError *)error {
    if (error.code != NSURLErrorCancelled) {
        [self retry];
    }
}

- (void)webViewWebContentProcessDidTerminate:(WKWebView *)webView {
    [webView reload];
}

/*
 * Decided by host. Anything on loopback stays inside. A link to anywhere else opens in the browser,
 * and anything else on a foreign host is refused.
 */
- (void)webView:(WKWebView *)webView
    decidePolicyForNavigationAction:(WKNavigationAction *)action
                    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    NSURL *url = action.request.URL;
    if (url == nil || km_is_ours(url)) {
        decisionHandler(WKNavigationActionPolicyAllow);
        return;
    }
    if (action.navigationType == WKNavigationTypeLinkActivated) {
        [UIApplication.sharedApplication openURL:url options:@{} completionHandler:nil];
    }
    decisionHandler(WKNavigationActionPolicyCancel);
}

/* A link that asks for a new window opens in this one, or in the browser if it leaves loopback. */
- (WKWebView *)webView:(WKWebView *)webView
    createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
               forNavigationAction:(WKNavigationAction *)action
                    windowFeatures:(WKWindowFeatures *)windowFeatures {
    NSURL *url = action.request.URL;
    if (url != nil && !km_is_ours(url)) {
        [UIApplication.sharedApplication openURL:url options:@{} completionHandler:nil];
    } else if (url != nil) {
        [webView loadRequest:action.request];
    }
    return nil;
}

/*
 * Opens the system file browser on packages.
 *
 * The chosen file goes to km_machine_open_package, and the machine installs it as it installs a
 * dropped file, by copying it into its packages folder. SDL's own route for an opened URL is a
 * method of its scene delegate, which nothing outside SDL can call. A copy of the file is asked for,
 * because the original can sit in a provider that grants access only while it is held open.
 */
- (void)choosePackage {
    [self expand:NO];
    UTType *type = [UTType typeWithIdentifier:KMPackageType];
    if (type == nil) {
        NSLog(@"karaokemachine: the system does not know %@, so no file browser opens", KMPackageType);
        return;
    }
    UIDocumentPickerViewController *picker =
        [[UIDocumentPickerViewController alloc] initForOpeningContentTypes:@[ type ] asCopy:YES];
    picker.delegate = self;
    picker.allowsMultipleSelection = NO;
    [_controller presentViewController:picker animated:YES completion:nil];
}

- (void)documentPicker:(UIDocumentPickerViewController *)controller
    didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls {
    NSURL *url = urls.firstObject;
    if (url == nil || !url.isFileURL) {
        return;
    }
    km_machine_open_package(url.fileSystemRepresentation);
}

@end

/* Kept for the life of the process: the views hold the shell as their target, and do not retain it. */
static KMPhoneShell *km_shell;

void km_phone_shell_configure(NSString *support) {
    km_support = [support copy];
}

void km_phone_shell_attach(void *window) {
    if (km_shell != nil || window == NULL) {
        return;
    }
    UIViewController *controller = ((__bridge UIWindow *)window).rootViewController;
    if (controller == nil) {
        NSLog(@"karaokemachine: SDL's window has no view controller, so the controls are not shown");
        return;
    }
    km_shell = [[KMPhoneShell alloc] initOver:controller];
}
