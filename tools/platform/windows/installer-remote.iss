; The Windows setup program for the remote alone: one program, no components, nothing configured.
;
; Compiled by tools/platform/windows/installer-remote.sh, which is the only thing that should invoke it
; -- that script stages the payload, reads the version out of the binary and passes the five defines
; below. Compiling this file by hand is possible and needs all five:
;
;   ISCC.exe /DPayload=C:\...\dist\km-remote\windows\km-remote-1.17.0-x86_64-pc-windows-msvc \
;            /DVersion=1.17.0 /DOutDir=C:\...\dist\setup\windows \
;            /DOutBase=km-remote-setup-1.17.0-windows-x86_64 \
;            /DGenerated=C:\...\dist\setup\windows\km-remote-generated \
;            tools\platform\windows\installer-remote.iss
;
; **This file names every file it installs**, and the driver reads the [Files] section back out and
; reconciles it against the staged folder -- so a file that arrives there and is not mentioned here
; fails the build rather than being silently left out. The same rule the all-in-one applies, and for
; the same reason: a carrier that quietly loses a file is the one failure neither of them may have.
;
; **The console twin is deliberately absent**, as it is from the all-in-one: `km-remote-console.exe`
; exists so that a Windows *folder* gives somebody something to type, and an installed program has a
; Start Menu entry instead. See the `What an installed build contains` decision in
; docs/decisions/distribution.md.
;
; **Its own AppId, its own folder, its own Start Menu group.** Inno makes a second run an upgrade of
; the first when the AppId matches and finds the uninstaller by it, so a shared one would mean
; installing this took the karaoke machine away. A computer may hold both, and removing either leaves
; the other. That is the `A setup program for the remote alone` decision.
;
; **Per-user, and here nothing else was ever on the table**: this configures nothing beyond the files
; -- no PATH entry, no file association -- so there is no account-versus-machine question to answer,
; and a per-user install raises no UAC prompt and lands where `winget` puts per-user software.

#ifndef Payload
  #error Payload is not defined. Run tools/platform/windows/installer-remote.sh; it stages the folder this needs.
#endif
#ifndef Version
  #error Version is not defined. installer-remote.sh reads it out of the staged binary.
#endif
#ifndef OutDir
  #error OutDir is not defined.
#endif
#ifndef OutBase
  #error OutBase is not defined.
#endif
#ifndef Generated
  #error Generated is not defined. installer-remote.sh writes the installed build's README there.
  #error It is not in the payload, because the payload's own README describes a folder.
#endif

#define AppName "KM Remote"
#define AppPublisher "Rangel Reale"
#define AppUrl "https://github.com/rrgmc/karaokemachine"

[Setup]
; **Never change AppId**, and never make it the karaoke machine's. It is what makes a second run an
; upgrade of the first rather than a second copy, what lets the uninstaller be found, and what keeps
; this carrier and the all-in-one from reaching into each other.
AppId={{3AA45469-E5BD-4EFD-9000-35EE90D6DE13}
AppName={#AppName}
AppVersion={#Version}
AppVerName={#AppName} {#Version}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
VersionInfoVersion={#Version}

; Per-user. No UAC prompt, ever -- `lowest` makes {autopf} resolve to {localappdata}\Programs and
; {group} to this user's own Start Menu.
PrivilegesRequired=lowest
; **A folder of its own rather than a child of the machine's.** Inno removes a directory it created
; if it is empty by the end of an uninstall, and [UninstallDelete] below reaches under {app}; a
; nested folder would put one carrier's uninstaller inside the other's tree. The folder and the group
; both follow AppName, which is why the driver asserts on that and on AppId and needs no third check.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
AllowNoIcons=yes

ArchitecturesAllowed=x64compatible
MinVersion=10.0

OutputDir={#OutDir}
OutputBaseFilename={#OutBase}
; Three levels up: this script lives at `tools/platform/windows/`, so `..\..\..` is the checkout.
SetupIconFile={#SourcePath}\..\..\..\icon\km-remote.ico
UninstallDisplayIcon={app}\km-remote.exe

; About 10 MB of payload, all of it one executable. **Not solid**: solid LZMA2 earns its compile
; minutes across the all-in-one's two large DLLs and its SoundFont, and across five small files it
; buys nothing.
Compression=lzma2/max
SolidCompression=no

; No ChangesEnvironment and no ChangesAssociations: this edits no PATH and claims no file type, so
; there is nothing for Windows to be told about.

WizardStyle=modern
DisableWelcomePage=no
DisableDirPage=no
DisableProgramGroupPage=no

; Inno finds files in use through the Restart Manager and offers to close them, which is what stops a
; mid-install failure naming one locked executable.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

; No [Types] and no [Components]: there is one program here, and a component page offering one tick
; is a page asking a question with one answer.
;
; No [Tasks] either. The all-in-one offers a PATH entry because it installs three commands that are
; useless if they cannot be typed, and a file association because the builder opens documents. This
; carries the windowed program alone, so both would configure something nothing here uses.

[Files]
Source: "{#Payload}\km-remote.exe"; DestDir: "{app}"; Flags: ignoreversion

; **The README an installed build gets**, in place of the payload's own folder document. That one
; says the folder holds the program and its console twin and tells you to remove it by deleting the
; folder, which is false beside an uninstaller and a Start Menu group. `dist_installed_readme` writes
; this one; see the block above the ISCC call in installer-remote.sh.
Source: "{#Generated}\README.txt"; DestDir: "{app}"; Flags: ignoreversion

; And the program's own document, which is right wherever it is read: what the remote is for, how to
; point it at a machine, and every option it takes. Renamed on the way in so it cannot be mistaken
; for the one above -- the same name the all-in-one installs it under.
Source: "{#Payload}\README.txt"; DestDir: "{app}"; DestName: "README-km-remote.txt"; Flags: ignoreversion

; The application's own terms. **An obligation rather than documentation**: MIT asks that the notice
; be included in every copy. There is no ffmpeg licence here because there is no ffmpeg -- the remote
; links none, and a notice for libraries a carrier does not hold would be a claim about nothing.
Source: "{#Payload}\LICENSE-MIT.txt";    DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\LICENSE-APACHE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}";           Filename: "{app}\km-remote.exe"
Name: "{group}\Read me first";        Filename: "{app}\README.txt"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"

[Run]
; The WebView2 bootstrapper, when [Code] decided one was needed and managed to fetch it. Per-user and
; silent, so this raises no prompt of its own.
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; \
  Check: WebView2WasFetched; StatusMsg: "Installing the Microsoft WebView2 runtime..."; Flags: runhidden

Filename: "{app}\km-remote.exe"; Description: "Start {#AppName} now"; \
  Flags: nowait postinstall skipifsilent unchecked

[UninstallDelete]
; Belt and braces, and kept for the reason the all-in-one keeps its own: WebView2 writes a profile of
; cache, cookies and logs beside any executable that opens a window without naming a cache directory,
; and the uninstaller removes only what it installed. The remote names a per-user directory, so on a
; current build this matches nothing -- what it guards against is any program that writes beside
; itself again, and an uninstaller leaving {app} standing is the kind of thing nobody notices.
Type: filesandordirs; Name: "{app}\*.WebView2"

[Code]

{ ---- WebView2 -------------------------------------------------------------------------------- }

{ Shared with the karaoke machine's setup program, which puts the same runtime behind the same
  window. The two sentences that differ are defines; the apostrophe in each is doubled because the
  text lands inside a Pascal string literal. }
#define WebView2Need "The Remote needs Microsoft''s WebView2 runtime, which is not on this computer."
#define WebView2Fallback "The Remote will open its page in your normal web browser instead of in a window of its own."
#include "webview2.iss"

{ No component to ask about: the one program this installs is the one that wants the runtime. }
function NeedsWebView2: Boolean;
begin
  Result := not WebView2Installed;
end;

{ ---- wiring ---------------------------------------------------------------------------------- }

procedure InitializeWizard;
begin
  CreateWebView2Page;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and NeedsWebView2 then
    FetchWebView2;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { Said once, at the end, because the thing people fear about an uninstaller is that it takes the
    collection with it. It does not, and where it is kept is not guessable. One folder, because this
    carrier installed one program. }
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    { No continuation line may begin with a `#`: ISPP reads one as a preprocessor directive, and the
      compile then fails on a line nobody wrote. }
    MsgBox('{#AppName} has been removed.' + #13#10#13#10 +
           'Your copy of the song list and your favorites have been left alone.' + #13#10 +
           'They are under:' + #13#10#13#10 +
           ExpandConstant('{userappdata}\km-remote') + #13#10#13#10 +
           'Delete that folder by hand if you want them gone.',
           mbInformation, MB_OK);
end;
