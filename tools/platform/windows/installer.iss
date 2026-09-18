; The Windows setup program: one installer carrying every product this repository builds.
;
; Compiled by tools/platform/windows/installer.sh, which is the only thing that should invoke it -- that
; script stages the payload, reads the version out of a binary and passes the five defines below.
; Compiling this file by hand is possible and needs all five:
;
;   ISCC.exe /DPayload=C:\...\dist\bin\windows /DVersion=1.17.0 /DOutDir=C:\...\dist\setup\windows \
;            /DOutBase=karaokemachine-setup-1.17.0-windows-x86_64 \
;            /DGenerated=C:\...\dist\setup\windows\generated tools\windows\installer.iss
;
; **This file names every file it installs.** installer.sh reads the [Files] section back out and
; reconciles it against what is actually in the payload, so a file that arrives in `dist/bin/windows`
; and is not mentioned here fails the build rather than being silently left out of the setup. That
; is the same rule tools/dist/bin.sh applies one level down, and for the same reason: a carrier that
; quietly loses a file is the one failure neither of them may have.
;
; **The console twins are deliberately absent.** `karaokemachine-console.exe` and its two siblings
; exist so that a Windows *folder* gives somebody something to type; an installed program has a Start
; Menu entry and a PATH instead, and a second executable differing from the first only in its
; subsystem is exactly the confusion an installer exists to remove. The portable folder and
; dist/bin-console still carry them. See the `What an installed build contains` decision in
; docs/decisions/distribution.md.
;
; **Per-user, because everything this installer configures beyond the files is per-user.** Both file
; associations are written by the programs themselves under HKCU\Software\Classes -- .kmbuild by
; km-package-builder and .kmpkg by the machine, each in its own register.rs, which is those files'
; decision and not this one -- and the PATH entry below is HKCU\Environment. A
; machine-wide install would therefore put files where every account can see them and configure them
; for exactly one, which is an incoherent install rather than a generous one. It also means no UAC
; prompt at any point, and it puts the programs where `winget` already puts per-user software.
;
; **This argument used to be a stronger and cruder one, and it is worth knowing it changed.** Until
; the `Where a webview keeps its profile` decision landed, WebView2 wrote its profile beside the
; executable that opened it, so {app} had to stay writable and Program Files was not merely a worse
; choice but an unusable one. Both windowed programs now name a per-user cache directory instead, so
; that constraint is gone; the conclusion is unchanged and the reason for it is not.

#ifndef Payload
  #error Payload is not defined. Run tools/platform/windows/installer.sh; it stages the folder this needs.
#endif
#ifndef Version
  #error Version is not defined. installer.sh reads it out of the staged binary.
#endif
#ifndef OutDir
  #error OutDir is not defined.
#endif
#ifndef OutBase
  #error OutBase is not defined.
#endif
#ifndef Generated
  #error Generated is not defined. installer.sh writes the installed build's README there.
  #error It is not in the payload, because the payload's own README describes a folder.
#endif

#define AppName "KaraokeMachine"
#define AppPublisher "Rangel Reale"
#define AppUrl "https://github.com/rrgmc/karaokemachine"

[Setup]
; **Never change AppId.** It is what makes a second run an upgrade of the first rather than a second
; copy, and what lets the uninstaller be found. The version moves; this does not.
AppId={{0B20FC71-00C6-41B2-A432-0F11DF8FE95F}
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
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
AllowNoIcons=yes

ArchitecturesAllowed=x64compatible
MinVersion=10.0

OutputDir={#OutDir}
OutputBaseFilename={#OutBase}
; Three levels up, not two: this script lives at `tools/platform/windows/`, and `..\..` from here is
; `tools/`. It was two while it sat at `tools/windows/`, and the move into six folders left it
; pointing at `tools/icon/`, which has never existed -- so the compile failed at this line with
; nothing but "the system cannot find the path specified" and no mention of an icon.
SetupIconFile={#SourcePath}\..\..\..\icon\karaokemachine.ico
UninstallDisplayIcon={app}\karaokemachine.exe

; About 200 MB of payload, three quarters of it ffmpeg and the instrument bank. Solid LZMA2 is worth
; the compile minutes here: the two big DLLs and the SoundFont compress well and a download is the
; thing a recipient waits on.
Compression=lzma2/max
SolidCompression=yes

; Both are declarations to Windows rather than behavior: the first makes Inno broadcast
; WM_SETTINGCHANGE after the PATH edit in [Code], the second refreshes the shell's icon cache after
; km-package-builder claims .kmbuild, and the machine claims .kmpkg.
ChangesEnvironment=yes
ChangesAssociations=yes

WizardStyle=modern
DisableWelcomePage=no
DisableDirPage=no
DisableProgramGroupPage=no

; Inno finds files in use through the Restart Manager and offers to close them, which is what stops a
; mid-install failure naming one locked executable. None of these programs registers a named mutex,
; so this is the mechanism that applies.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Types]
Name: "full"; Description: "Everything"
Name: "machine"; Description: "Just KaraokeMachine"
Name: "custom"; Description: "Choose what to install"; Flags: iscustom

[Components]
Name: "machine"; Description: "KaraokeMachine -- plays the songs"; Types: full machine; Flags: checkablealone
Name: "builder"; Description: "KM Package Builder -- turns a folder of songs into a package"; Types: full
Name: "remote"; Description: "KM Remote -- search and queue from this computer"; Types: full
Name: "assets"; Description: "KM Admin -- find pictures and instrument banks for the machine"; Types: full
Name: "tools"; Description: "Command-line tools (km-pack, km-lyrics, km-wallpaper-pack)"; Types: full

[Tasks]
; Ticked by default: the three command-line tools are useless if they cannot be typed, and the two
; windowed programs are harmless on a PATH. Removal on uninstall is in [Code] -- Inno has no
; declarative form for taking one entry back out of a value it did not create.
Name: "addpath"; Description: "Add the installation folder to my PATH"; GroupDescription: "Set up:"
; A Task rather than an unconditional [Run], so that it can be declined and so that a silent install
; with /TASKS="" changes nothing outside {app}.
Name: "assoc"; Description: "Open .kmbuild corpus files with the Package Builder"; GroupDescription: "Set up:"; Components: builder
; The machine's own, and a separate task rather than a second thing the one above does: the two
; components are separately installable, so somebody who took the machine and declined the builder
; must still be offered this one, and somebody who took both must be able to decline either.
Name: "assocpkg"; Description: "Open .kmpkg song packages with KaraokeMachine"; GroupDescription: "Set up:"; Components: machine
Name: "desktopicon"; Description: "Create a desktop shortcut for KaraokeMachine"; GroupDescription: "Set up:"; Components: machine; Flags: unchecked
; **This installer does not download the bank; it writes down that you asked for it.** The machine
; fetches it on its first start, from the pinned URL against the pinned digest, with a line on the
; television saying how far it has got -- so an install stays as fast and as offline-safe as it was,
; and a 261.9 MiB file does not have to travel inside a 30 MiB carrier. See `Offering the recommended
; bank at install time` in docs/decisions/distribution.md.
;
; Ticked by default, unlike `desktopicon`: the bundled bank is the one thing about a fresh install
; that is measurably not the best available, and somebody who does not know what a SoundFont is is
; exactly who this is for. The size is in the description and the terms are on the Ready page, so it
; is declined by somebody who has been told what it costs rather than skipped in ignorance.
Name: "soundfont"; Description: "Download the recommended instrument bank ({#BankSize}) when {#AppName} first starts"; GroupDescription: "Set up:"; Components: machine

[Files]
; -- the programs. One per component; no console twins, see the header. ---------------------------
Source: "{#Payload}\karaokemachine.exe";     DestDir: "{app}"; Components: machine; Flags: ignoreversion
Source: "{#Payload}\km-package-builder.exe"; DestDir: "{app}"; Components: builder; Flags: ignoreversion
Source: "{#Payload}\km-remote.exe";      DestDir: "{app}"; Components: remote;  Flags: ignoreversion
Source: "{#Payload}\km-admin.exe";      DestDir: "{app}"; Components: assets;  Flags: ignoreversion
Source: "{#Payload}\km-pack.exe";            DestDir: "{app}"; Components: tools;   Flags: ignoreversion
Source: "{#Payload}\km-lyrics.exe";          DestDir: "{app}"; Components: tools;   Flags: ignoreversion
Source: "{#Payload}\km-wallpaper-pack.exe";     DestDir: "{app}"; Components: tools;   Flags: ignoreversion

; -- the instrument bank, the wallpapers and the font ---------------------------------------------
;
; Only with the machine, which is the component that reads them -- this is where somebody who wanted
; only the remote stops paying for 52 MB. They have to land *beside* the exe:
; `Paths::discover_asset_dir` in crates/machine/karaokemachine/src/settings.rs takes the directory holding the
; binary when it has an `assets` child, and a machine that cannot find them comes up on a sine test
; tone over a plain gradient with nothing on screen saying why.
Source: "{#Payload}\assets\*"; DestDir: "{app}\assets"; Components: machine; \
  Flags: ignoreversion recursesubdirs createallsubdirs

; -- and the bank the tick box asks for ------------------------------------------------------------
;
; Two lines of JSON naming one bank, written where the machine keeps its settings rather than beside
; the executable: this is a request the machine consumes and deletes, and {app} is not a place it
; may write on a machine installed for every user. installer.sh generates it from the bank table, so
; the id here and the size in the tick box cannot come to disagree.
;
; `onlyifdoesntexist` because a reinstall over a machine that is already mid-request must not reset
; the attempt count the machine has been writing into it, and `uninsneveruninstall` for the reason
; the packages folder below has it -- the uninstaller's closing promise that settings were left alone
; has to be true of everything in that folder, including a request that never got carried out.
Source: "{#Generated}\first-run-soundfont.json"; DestDir: "{userappdata}\karaokemachine\config"; \
  Tasks: soundfont; Flags: onlyifdoesntexist uninsneveruninstall

; -- the settings file, where there is not one already ---------------------------------------------
;
; Two keys: `display.fullscreen`, true. That is what makes an installed machine drive a television,
; and it is here rather than in the binary's defaults because a *default* cannot tell an install from
; a checkout somebody has just built -- both have no settings file, and defaulting on took the whole
; screen away from everybody debugging it.
;
; Every settings struct is `#[serde(default)]`, so two keys are a whole settings file: the machine
; fills the rest in and writes it back complete on the first start.
;
; **`onlyifdoesntexist` is the whole of the safety argument, and it is not the same use of the flag
; as the line above.** There it stops a reinstall resetting an attempt count. Here it is what keeps
; this from being the thing firstrun.rs refuses -- an installer that overwrites a settings file it
; did not create, losing whatever an upgrade was standing on. With it, this writes only where it is
; creating the install, and an upgrade over a machine somebody has been using changes nothing.
; `uninsneveruninstall` for the same reason as the line above: the uninstaller's closing promise that
; settings were left alone has to be true of the settings file most of all.
;
; No `Tasks:` -- unlike the SoundFont, this is not something somebody ticked. It is what kind of
; install this is.
Source: "{#Generated}\settings.json"; DestDir: "{userappdata}\karaokemachine\config"; \
  Flags: onlyifdoesntexist uninsneveruninstall

; -- ffmpeg ---------------------------------------------------------------------------------------
;
; Exactly the four libraries the executables import; the list is owned by DIST_FFMPEG_DLLS in
; tools/dist/common.sh and installer.sh checks that this section and that array still agree.
;
; Beside the exes rather than in a lib/ subfolder, because the Windows loader searches the directory
; the executable was loaded from and nowhere else that would help. Named by three components at once
; (the machine, the builder and the CLI tools all link them); Inno installs such a file once.
Source: "{#Payload}\avcodec-61.dll";    DestDir: "{app}"; Components: machine builder tools; Flags: ignoreversion
Source: "{#Payload}\avformat-61.dll";   DestDir: "{app}"; Components: machine builder tools; Flags: ignoreversion
Source: "{#Payload}\avutil-59.dll";     DestDir: "{app}"; Components: machine builder tools; Flags: ignoreversion
Source: "{#Payload}\swresample-5.dll";  DestDir: "{app}"; Components: machine builder tools; Flags: ignoreversion
; The LGPL obligation: the terms travel with the binaries. Unconditional, so that it cannot go
; missing from a build whose component selection happened to drop every DLL user.
Source: "{#Payload}\ffmpeg-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

; The application's own terms, on exactly the same footing and for the same reason: MIT asks that the
; notice be included in every copy, so this is an obligation rather than documentation. Unconditional,
; because every component is covered by it.
Source: "{#Payload}\LICENSE-MIT.txt";    DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\LICENSE-APACHE.txt"; DestDir: "{app}"; Flags: ignoreversion

; -- what each program is, in the folder rather than in a repository ------------------------------
; Always installed, whatever was selected: they are a few kilobytes and they are how somebody finds
; out that the other components exist.
;
; **The first one comes from {#Generated} and not from the payload**, and that is the whole of the
; difference between an installed build and a folder somebody unzipped. The payload's own README.txt
; describes the folder tools/dist/bin.sh gathers -- every executable this platform can build, remove
; it by deleting the folder, nothing was registered -- none of which survives being installed by
; component tick beside a Start Menu group, a PATH entry and a file association. It is generated by
; `dist_installed_readme` in tools/dist/common.sh, shared with the macOS package. The payload's copy
; is left where it is and named as a deliberate exclusion in installer.sh's coverage check.
Source: "{#Generated}\README.txt";   DestDir: "{app}"; Flags: ignoreversion
Source: "{#Payload}\README-*.txt";   DestDir: "{app}"; Flags: ignoreversion

[Dirs]
; The songs folder, made here rather than waiting for the machine's first start -- because the
; shortcut below has to point at something that exists, and because "where do I put my songs?" is a
; question somebody has before they have run anything. The machine creates it too and does not mind
; finding it already there. Under {userappdata} and not {app}: it is the owner's own accumulating
; files, which is the `Where packages live` decision in docs/decisions/, and it deliberately survives
; the uninstall along with the settings and the catalog.
; `uninsneveruninstall` so that the uninstaller's closing promise -- "your songs, settings and
; catalog have been left alone" -- is true of this folder too. Without it Inno removes a directory
; it created if it is empty by then, which is a small thing that would make that sentence a lie.
Name: "{userappdata}\karaokemachine\data\packages"; Components: machine; Flags: uninsneveruninstall

[Icons]
Name: "{group}\KaraokeMachine";                 Filename: "{app}\karaokemachine.exe";     Components: machine
; **The same machine, drawing for an encoder instead of for a television.** `--stream` is a way of
; running it rather than a build of it -- it overrides `display.enabled` for this process and writes
; nothing back -- so the two entries are one program, and an install started this way once still
; opens its television next time.
;
; This is the only launcher here that passes an argument, and it is what makes the mode reachable
; without a command line. The run it starts has no window and, being the GUI-subsystem executable, no
; console either: what says it is there is the icon in the notification area, whose menu carries the
; three pages it serves.
;
; **`IconIndex: 1` is the badged mark, out of the executable the line already names.** karaokemachine.exe
; carries two icon resources -- crates/machine/karaokemachine/build.rs attaches them -- and an icon
; index counts them from zero in resource order, so 0 is the machine and 1 is the machine streaming.
; Taking it from the exe rather than installing a second .ico is the arrangement `km_webshell::with_icons`
; and the notification area both already use: there is one copy of the picture and nothing to keep in
; step with it. `IconFilename` has to be given for `IconIndex` to be read at all, which is why it names
; the file the shortcut points at anyway.
Name: "{group}\KaraokeMachine (stream)";        Filename: "{app}\karaokemachine.exe";     Parameters: "--stream"; \
  IconFilename: "{app}\karaokemachine.exe"; IconIndex: 1; Components: machine
; **The answer to "where do I put my songs?" on a machine with no console.** `--show-paths` prints
; the folder, and that is no use to somebody who installed this by double-clicking a setup program
; and drives it from a sofa; the idle screen deliberately will not put a Windows path on a
; television. A Start Menu entry beside the machine itself is the platform's own way of saying it,
; and opens the folder in Explorer so a .kmpkg can be dropped straight in.
Name: "{group}\Karaoke songs folder";           Filename: "{userappdata}\karaokemachine\data\packages"; Components: machine
Name: "{group}\KM Package Builder"; Filename: "{app}\km-package-builder.exe"; Components: builder
Name: "{group}\KM Remote";          Filename: "{app}\km-remote.exe";      Components: remote
Name: "{group}\KM Admin";          Filename: "{app}\km-admin.exe";      Components: assets
Name: "{group}\Read me first";                  Filename: "{app}\README.txt"
Name: "{group}\Uninstall {#AppName}";           Filename: "{uninstallexe}"
Name: "{autodesktop}\KaraokeMachine";           Filename: "{app}\karaokemachine.exe";     Components: machine; Tasks: desktopicon

[Run]
; **The association is made by the program, not by this installer**, and that is deliberate:
; tools/cmd/km-package-builder/src/register.rs already owns the four values under HKCU\Software\Classes
; and takes the icon out of the executable's own resources. A second copy of that here is exactly the
; drift this repository writes single definitions to avoid.
;
; Calling it on the *windowed* executable is correct: `windowed_twin_of` returns None for a name with
; no `-console` suffix and `executable()` then falls back to the exe itself, so the GUI binary
; registers the GUI binary -- which is the one a double-click should reach.
;
; No `nowait`: it is instant, and a GUI-subsystem program has nowhere to print a failure, so the only
; thing that could notice one is the exit code Inno collects here.
Filename: "{app}\km-package-builder.exe"; Parameters: "--register"; Tasks: assoc; \
  StatusMsg: "Associating .kmbuild files..."; Flags: runhidden

; The same arrangement for the machine's own document type, and the same reasoning line for line --
; crates/machine/karaokemachine/src/register.rs owns the values. Note this names the *windowed*
; karaokemachine.exe rather than the console twin, which is the point of `windowed_twin_of`: a
; double-clicked package must not open a console window.
Filename: "{app}\karaokemachine.exe"; Parameters: "--register"; Tasks: assocpkg; \
  StatusMsg: "Associating .kmpkg files..."; Flags: runhidden

; The WebView2 bootstrapper, when [Code] decided one was needed and managed to fetch it. Per-user and
; silent, so this raises no prompt of its own.
Filename: "{tmp}\MicrosoftEdgeWebview2Setup.exe"; Parameters: "/silent /install"; \
  Check: WebView2WasFetched; StatusMsg: "Installing the Microsoft WebView2 runtime..."; Flags: runhidden

Filename: "{app}\karaokemachine.exe"; Description: "Start KaraokeMachine now"; \
  Components: machine; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
Filename: "{app}\km-package-builder.exe"; Parameters: "--unregister"; Tasks: assoc; \
  RunOnceId: "unregister_kmbuild"; Flags: runhidden
; A RunOnceId of its own, and it has to be: Inno runs each id at most once per uninstall, so
; sharing one would silently skip whichever of the two came second.
Filename: "{app}\karaokemachine.exe"; Parameters: "--unregister"; Tasks: assocpkg; \
  RunOnceId: "unregister_kmpkg"; Flags: runhidden

[UninstallDelete]
; **Kept after the fault it was written for was fixed, and downgraded from load-bearing to
; belt-and-braces.** WebView2 used to create `<exe name>.WebView2\` beside the executable that opened
; a window -- a browser profile of cache, cookies and logs -- so an uninstall left {app} standing
; with a profile in it, since Inno removes only what it installed. Both windowed programs now name a
; per-user cache directory (`Where a webview keeps its profile` in docs/decisions/interface.md), so on a current
; build nothing writes here at all and this line matches nothing.
;
; It stays because it costs one line and its failure mode is asymmetric: what it guards against is
; any program that ever writes beside itself again, and the uninstaller leaving {app} behind is the
; kind of thing nobody notices until they look. It is **not** kept for migration -- nothing has been
; released, so there is no installed build in the world carrying an old profile folder.
;
; The round trip in installer.sh plants such a folder and asserts it is gone, so what is actually
; being tested is that the uninstaller removes a directory it did not install. That property is
; worth holding whether or not anything currently creates one.
Type: filesandordirs; Name: "{app}\*.WebView2"

[Code]

const
  EnvironmentKey = 'Environment';

{ ---- WebView2 -------------------------------------------------------------------------------- }

{ Shared with the remote's own setup program, which puts the same runtime behind the same window.
  The two sentences that differ are defines; the apostrophe in each is doubled because the text lands
  inside a Pascal string literal. }
#define WebView2Need "The Package Builder and the Remote need Microsoft''s WebView2 runtime, which is not on this computer."
#define WebView2Fallback "The Package Builder and the Remote will open their pages in your normal web browser instead of in a window of their own."
#include "webview2.iss"

{ Only the two programs that put a webview in a window care. The machine draws with SDL and the
  command-line tools have no window at all, so an install of those alone must not reach the network. }
function NeedsWebView2: Boolean;
begin
  Result := (WizardIsComponentSelected('builder') or WizardIsComponentSelected('remote')) and not WebView2Installed;
end;

{ ---- PATH ------------------------------------------------------------------------------------ }

{ Both directions live here rather than half in [Registry], so that the add and the remove cannot
  drift apart -- and because taking one entry back out of a value this installer did not create has
  no declarative form at all.

  Read with RegQueryStringValue and written back with RegWriteExpandStringValue: the user's Path
  routinely contains %USERPROFILE% and friends, and rewriting it as a plain string would expand them
  permanently. Inno's Pascal strings have no length limit, so a long real-world Path survives. }

function PathContains(const Haystack, Needle: String): Boolean;
begin
  Result := Pos(';' + Uppercase(Needle) + ';', ';' + Uppercase(Haystack) + ';') > 0;
end;

procedure AddToPath;
var
  Existing: String;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Existing) then
    Existing := '';
  if PathContains(Existing, ExpandConstant('{app}')) then
    exit;
  if Existing = '' then
    Existing := ExpandConstant('{app}')
  else if Copy(Existing, Length(Existing), 1) = ';' then
    Existing := Existing + ExpandConstant('{app}')
  else
    Existing := Existing + ';' + ExpandConstant('{app}');
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Existing);
end;

procedure RemoveFromPath;
var
  Existing, Rebuilt, Part: String;
  P: Integer;
  Target: String;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Existing) then
    exit;
  Target := Uppercase(ExpandConstant('{app}'));
  Rebuilt := '';
  { Split on ';' and keep every entry that is not ours. Only the entry this installer added is
    dropped, matched whole -- a substring match would take a sibling folder with a longer name. }
  while Existing <> '' do
  begin
    P := Pos(';', Existing);
    if P = 0 then
    begin
      Part := Existing;
      Existing := '';
    end
    else
    begin
      Part := Copy(Existing, 1, P - 1);
      Existing := Copy(Existing, P + 1, Length(Existing));
    end;
    if (Part <> '') and (Uppercase(Part) <> Target) then
    begin
      if Rebuilt = '' then
        Rebuilt := Part
      else
        Rebuilt := Rebuilt + ';' + Part;
    end;
  end;
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Rebuilt);
end;

{ ---- wiring ---------------------------------------------------------------------------------- }

procedure InitializeWizard;
begin
  CreateWebView2Page;
end;

{ **The bank's terms, on the page somebody reads before agreeing.** A tick box holds one line, and
  the recommended bank's license is one this project rates doubtful -- so recommending it is only
  defensible with the terms beside it, which is what `Where a bank may be fetched from` in
  docs/decisions/repository.md requires of every offer. The Ready page is where Inno already
  summarizes what was chosen, so the sentence lands next to the decision rather than in a dialog
  nobody asked for.

  Appended to the memo Inno built rather than replacing it: MemoTasksInfo already lists the tick box
  itself, and this says what the tick box costs. }
function UpdateReadyMemo(const Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo,
  MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := MemoDirInfo + NewLine + MemoTypeInfo + NewLine + MemoComponentsInfo + NewLine +
            MemoGroupInfo + NewLine + MemoTasksInfo;
  if WizardIsTaskSelected('soundfont') then
    Result := Result + NewLine + NewLine +
      'Instrument bank to download on first start:' + NewLine +
      Space + '{#BankName} ({#BankSize})' + NewLine +
      Space + 'License: {#BankLicense}' + NewLine +
      Space + 'It is downloaded by KaraokeMachine itself, from the publisher, not by Setup.';
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and NeedsWebView2 then
    FetchWebView2;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addpath') then
    AddToPath;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromPath;

  { Said once, at the end, because the thing people fear about an uninstaller is that it takes the
    songs with it. It does not, and where they are is not guessable. }
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    MsgBox('{#AppName} has been removed.' + #13#10#13#10 +
           'Your songs, settings and catalog have been left alone. They are under:' + #13#10#13#10 +
           ExpandConstant('{userappdata}\karaokemachine') + #13#10 +
           ExpandConstant('{userappdata}\km-package-builder') + #13#10 +
           ExpandConstant('{userappdata}\km-remote') + #13#10 +
           ExpandConstant('{userappdata}\km-admin') + #13#10#13#10 +
           'Any .kmbuild file in a folder of songs, and any .kmpkg package, has been left alone ' +
           'too. Delete these by hand if you want them gone.',
           mbInformation, MB_OK);
end;
