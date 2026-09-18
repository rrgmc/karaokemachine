// The WebView2 runtime: is it here, and if not, fetch Microsoft's bootstrapper during the install.
//
// Included into the [Code] section of every setup program that installs a windowed program with a
// webview in it. Two sentences differ between carriers and are taken as defines, which the including
// script sets before the #include:
//
//   WebView2Need      why the page is on screen, e.g. "The Remote needs Microsoft's WebView2 runtime,
//                     which is not on this computer."
//   WebView2Fallback  what happens if the download fails, e.g. "The Remote will open its page in your
//                     normal web browser instead of in a window of its own."
//
// The including script keeps three short things of its own, because each is a sentence about that
// carrier rather than about WebView2: `NeedsWebView2`, which asks whether anything being installed
// wants it; `InitializeWizard`, which calls `CreateWebView2Page`; and `NextButtonClick`, which calls
// `FetchWebView2` on the Ready page.
//
// **A failed download is not a failed install.** A build whose webview cannot be created opens its
// pages in the ordinary browser instead, so this is a convenience rather than a prerequisite -- see
// the `The package builder's window` decision in docs/decisions/interface.md.

#ifndef WebView2Need
  #error WebView2Need is not defined. Set it before including webview2.iss.
#endif
#ifndef WebView2Fallback
  #error WebView2Fallback is not defined. Set it before including webview2.iss.
#endif

// Microsoft's evergreen bootstrapper is tiny and installs per-user without elevation; the GUID is the
// WebView2 runtime's own registration under EdgeUpdate and is a Microsoft-published constant, not
// something to regenerate.
#define WebView2Url "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
#define WebView2Guid "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

var
  WebView2Page: TDownloadWizardPage;
  WebView2Fetched: Boolean;

{ The runtime registers a `pv` under EdgeUpdate. Three places are checked because Setup is a 32-bit
  process: HKLM reads are redirected into WOW6432Node, so the explicit 64-bit view is asked as well,
  and a per-user runtime lives under HKCU. `0.0.0.0` is what a stale registration left by an
  uninstall looks like, and it means the runtime is not there. }
function WebView2Installed: Boolean;
var
  Version: String;
  Path: String;
begin
  Path := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{#WebView2Guid}';
  Result :=
    (RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{#WebView2Guid}', 'pv', Version)
      and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKLM, Path, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0')) or
    (RegQueryStringValue(HKCU, Path, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0'));
end;

function WebView2WasFetched: Boolean;
begin
  Result := WebView2Fetched;
end;

function OnWebView2Progress(const Url, Filename: String; const Progress, ProgressMax: Int64): Boolean;
begin
  Result := True;
end;

procedure CreateWebView2Page;
begin
  WebView2Fetched := False;
  WebView2Page := CreateDownloadPage('Downloading the WebView2 runtime', '{#WebView2Need}', @OnWebView2Progress);
end;

{ Say what did not happen, name the URL, and carry on installing. }
procedure FetchWebView2;
begin
  WebView2Page.Clear;
  WebView2Page.Add('{#WebView2Url}', 'MicrosoftEdgeWebview2Setup.exe', '');
  WebView2Page.Show;
  try
    try
      WebView2Page.Download;
      WebView2Fetched := True;
    except
      MsgBox('The WebView2 runtime could not be downloaded:' + #13#10#13#10 +
             GetExceptionMessage + #13#10#13#10 +
             'Setup will carry on. {#WebView2Fallback}' + #13#10#13#10 +
             'To fix it later, install the runtime from:' + #13#10 + '{#WebView2Url}',
             mbInformation, MB_OK);
    end;
  finally
    WebView2Page.Hide;
  end;
end;
