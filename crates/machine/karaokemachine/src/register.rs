//! Telling the operating system what a `.kmpkg` file is.
//!
//! `--register` associates the extension with *this* executable, wherever it currently is, and
//! `--unregister` takes it back off. A double-clicked package then installs itself, which is the
//! shortest route into the catalog there is: no path to know, no folder to find, no window to
//! drag onto.
//!
//! **Per-user, never machine-wide.** On Windows that means `HKEY_CURRENT_USER`, which needs no
//! elevation; on Linux, `XDG_DATA_HOME` and its `~/.local/share` default, which needs no root. A
//! program that asked for administrator rights to make double-clicking work would be a worse trade
//! than typing the path.
//!
//! **The Linux arm is what the tarball's `install.sh` runs**, rather than a second implementation
//! of the same three files beside it. The two disagreed while there were two, and the shell's copy
//! declared no MIME type at all — so the entry it wrote claimed a type the desktop had never heard
//! of and never read the line that said so.
//!
//! **macOS does it differently and does not come through here at all.** A document type is declared
//! in a bundle's `Info.plist` — `CFBundleDocumentTypes` plus an exported UTI — and LaunchServices
//! picks it up when the `.app` is placed or launched. There is nothing for a command to write, so
//! `--register` there only nudges LaunchServices at a bundle that already declares everything.
//!
//! **Android and iOS declare the type in their own manifests**, so there is nothing for a command
//! to write and no way to ask for one; both get an arm that refuses. That arm is not decoration:
//! without it the Android build does not compile, because the module below is three `#[cfg]`s that
//! a fourth target matches none of.
//!
//! # Why this is a copy of `km-package-builder`'s and not a shared crate
//!
//! Because this is the **second** caller, and `km-osopen`'s own header records the rule this
//! repository set for exactly this situation: a third caller earns the crate. It is worth being
//! precise about how much is actually shared, because "two files that look alike" is not the same
//! as "one file written twice". Everything that varies varies: the extension, the ProgID, the MIME
//! type, the UTI, the description, the icon set, the desktop entry's category list, and what the
//! program *does* with the file it is handed — the builder opens a corpus, this installs a package
//! and may hand it to a machine already running. What is genuinely identical is
//! [`windowed_twin_of`], which is copied verbatim and is nine lines.
//!
//! The third caller would be the offline remote, which has no document type at all — so it would be
//! a new thing rather than a third of these, and inventing `crates/platform/km-fileassoc` for two
//! would have been inventing it for one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The icon-theme name the desktop entry points at, and the basename each PNG is installed under.
///
/// One string for the file in `icon/`, the `Icon=` line and the installed path, so a rename cannot
/// leave two of the three agreeing. It is the machine's own name because a `.kmpkg` in a file
/// manager should look like the thing that opens it.
#[cfg(target_os = "linux")]
const ICON_NAME: &str = "karaokemachine";

/// What the desktop entry's stream action names, on the same terms as [`ICON_NAME`].
///
/// A theme name rather than a path, so the `.deb`'s copies and `--register`'s land in the same
/// place under the same name and the second is a no-op over the first.
#[cfg(target_os = "linux")]
const STREAM_ICON_NAME: &str = "karaokemachine-stream";

/// The desktop entry, compiled in from the file the `.deb` installs.
///
/// **The same bytes reach both routes, so a per-user entry and a system one cannot disagree.**
/// `Exec=` is rewritten on the way out and everything else is carried through, which is what keeps
/// `StartupWMClass`, `Keywords` and `GenericName` — all of which a hand-written copy here lost, and
/// the first of those is what ties a running window back to this entry's icon.
#[cfg(target_os = "linux")]
const DESKTOP_ENTRY: &str = include_str!("../linux/karaokemachine.desktop");

/// What a `.kmpkg` is, compiled in from the file the `.deb` installs.
///
/// Embedded rather than written out for [`DESKTOP_ENTRY`]'s reason, and it is the half that is easy
/// to leave out: the entry above says this program opens the type and this says the type exists,
/// and a desktop that has not been told the second never reads the first.
#[cfg(target_os = "linux")]
const MIME_DEFINITION: &str = include_str!("../linux/karaokemachine-package.xml");

/// The line of [`DESKTOP_ENTRY`] that names the program, as the shipped file spells it.
///
/// The `.deb` puts a symlink in `/usr/bin`, so the file it installs names the command and nothing
/// else. Every other way in has the executable somewhere of its own choosing, which is what
/// [`platform::desktop_entry_for`] rewrites this to.
#[cfg(target_os = "linux")]
const PACKAGED_EXEC: &str = "Exec=karaokemachine";

/// One mark, as the sizes it was rendered at: (pixels, the PNG).
#[cfg(target_os = "linux")]
type IconSet = &'static [(u32, &'static [u8])];

/// The machine's icon at each size a desktop might ask for, compiled in.
///
/// **Embedded rather than read from disk**, because `--register` is run by somebody who unpacked a
/// tarball wherever they liked, and a path to an icon file is one more thing a move breaks. 68 KB.
///
/// Note these are `icon/icon-*.png`, which is the machine's amber mark — the same files the `.deb`
/// installs under this exact theme name. A `.deb` install and a `--register` therefore agree rather
/// than fighting, and the second is a no-op over the first.
#[cfg(target_os = "linux")]
const ICON_PNGS: IconSet = &[
    (16, include_bytes!("../../../../icon/icon-16.png")),
    (32, include_bytes!("../../../../icon/icon-32.png")),
    (48, include_bytes!("../../../../icon/icon-48.png")),
    (64, include_bytes!("../../../../icon/icon-64.png")),
    (128, include_bytes!("../../../../icon/icon-128.png")),
    (256, include_bytes!("../../../../icon/icon-256.png")),
    (512, include_bytes!("../../../../icon/icon-512.png")),
];

/// The badged mark, at each size a menu might ask for, compiled in on the same terms.
///
/// Six where the mark above gets seven: this one is named by the desktop entry's stream action and
/// an action is drawn in a menu, which never asks for 512. 44 KB.
#[cfg(target_os = "linux")]
const STREAM_ICON_PNGS: IconSet = &[
    (
        16,
        include_bytes!("../../../../icon/karaokemachine-stream-16.png"),
    ),
    (
        32,
        include_bytes!("../../../../icon/karaokemachine-stream-32.png"),
    ),
    (
        48,
        include_bytes!("../../../../icon/karaokemachine-stream-48.png"),
    ),
    (
        64,
        include_bytes!("../../../../icon/karaokemachine-stream-64.png"),
    ),
    (
        128,
        include_bytes!("../../../../icon/karaokemachine-stream-128.png"),
    ),
    (
        256,
        include_bytes!("../../../../icon/karaokemachine-stream-256.png"),
    ),
];

/// Both marks, each under the theme name that finds it.
///
/// **One list rather than two loops**, so registering and unregistering cannot come to disagree
/// about how many marks there are — which is the failure that leaves a mark behind in every
/// application menu after an uninstall.
#[cfg(target_os = "linux")]
const ICON_SETS: &[(&str, IconSet)] =
    &[(ICON_NAME, ICON_PNGS), (STREAM_ICON_NAME, STREAM_ICON_PNGS)];

/// What the console twin is called, next to the windowed executable it belongs to. See
/// [`windowed_twin_of`].
const CONSOLE_SUFFIX: &str = "-console";

/// The windowed executable beside this one, when this one is the console twin.
///
/// **Registering has to name the program a double-click should reach, and that is never this one.**
/// `--set-password` and `--show-paths` live on `karaokemachine-console` — `cli.rs` says as much —
/// so that is the executable somebody is most likely to be typing when they run `--register`. A
/// `.kmpkg` associated with it would open a console window on a double-click, on the one platform
/// where the whole point of the windowed twin is that it does not.
///
/// `None` for the windowed executable itself, and for a console twin whose partner is not there —
/// somebody who copied one file out of a staged folder gets themselves registered, which works and
/// is what they asked for.
fn windowed_twin_of(exe: &Path) -> Option<PathBuf> {
    let windowed = exe.file_stem()?.to_str()?.strip_suffix(CONSOLE_SUFFIX)?;
    let mut sibling = exe.with_file_name(windowed);
    // Kept rather than assumed: `.exe` on Windows, nothing on the platforms that have no twin.
    if let Some(extension) = exe.extension() {
        sibling.set_extension(extension);
    }
    sibling.is_file().then_some(sibling)
}

/// Strips the verbatim prefix a Windows `canonicalize` leaves behind.
///
/// `\\?\C:\Program Files\…` is a perfectly valid path and a terrible thing to write into a registry
/// value somebody may read: Explorer copes, a person reading `regedit` wonders what went wrong.
fn tidy(path: &Path) -> PathBuf {
    let Ok(canonical) = path.canonicalize() else {
        return path.to_path_buf();
    };
    match canonical.to_string_lossy().strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => canonical,
    }
}

/// Where the executable to register is, tidied.
///
/// **`current_exe` does not resolve symlinks everywhere.** On Apple platforms `std` is
/// `_NSGetExecutablePath` and nothing else — no `realpath` — so through a symlink this records the
/// symlink. Harmless here, and a note rather than a fix: the macOS arm below refuses anything that
/// is not inside a bundle, and a symlink into `Contents/MacOS` is one of the shapes it refuses.
/// Windows and Linux, where a written-down path is what `--register` is *for*, do canonicalise.
fn executable() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding this executable")?;
    let exe = windowed_twin_of(&exe).unwrap_or(exe);
    Ok(tidy(&exe))
}

/// Associates `.kmpkg` with this executable.
pub fn register() -> Result<()> {
    let exe = executable()?;
    platform::register(&exe)?;
    km_console::say(format!("  registered  .kmpkg opens with {}", exe.display()));
    km_console::say("  Double-click a package to install it.");
    Ok(())
}

/// Takes the association back off.
pub fn unregister() -> Result<()> {
    platform::unregister()?;
    km_console::say("  unregistered  .kmpkg is no longer associated with this machine");
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::*;

    use windows_registry::CURRENT_USER;

    /// The class name the extension points at.
    ///
    /// A `Vendor.Product` name rather than something like `kmpkgfile`, which is the convention and
    /// which also makes it obvious in `regedit` who put it there. Deliberately not the builder's
    /// `KaraokeMachine.PackageBuilder`: two extensions, two classes, and unregistering one must not
    /// take the other's icon with it.
    const PROG_ID: &str = "KaraokeMachine.Package";

    /// Writes the three keys Explorer needs, under `HKCU\Software\Classes`.
    ///
    /// That hive is merged over the machine-wide one for this user, so it needs no elevation and
    /// cannot affect anybody else who logs in.
    pub fn register(exe: &Path) -> Result<()> {
        let classes = CURRENT_USER
            .create("Software\\Classes")
            .context("opening HKCU\\Software\\Classes")?;

        // The extension points at the class...
        let extension = classes
            .create(".kmpkg")
            .context("creating the .kmpkg key")?;
        extension
            .set_string("", PROG_ID)
            .context("naming the class for .kmpkg")?;

        // ...and the class says what it is called, what it looks like, and how to open it.
        let class = classes.create(PROG_ID).context("creating the class key")?;
        class
            .set_string("", "Karaoke song package")
            .context("naming the class")?;

        // The icon comes out of the executable's own resource, which `build.rs` already attaches --
        // so a `.kmpkg` in Explorer gets the machine's amber mark with nothing extra to ship or to
        // lose, and follows `build.rs` automatically if the mark ever changes.
        let icon = class
            .create("DefaultIcon")
            .context("creating DefaultIcon")?;
        icon.set_string("", format!("\"{}\",0", exe.display()))
            .context("setting the document icon")?;

        let command = class
            .create("shell\\open\\command")
            .context("creating the open command")?;
        // `"%1"` quoted, because a package path with a space in it is the normal case rather than
        // the exotic one, and an unquoted one arrives split across several arguments.
        command
            .set_string("", format!("\"{}\" \"%1\"", exe.display()))
            .context("setting the open command")?;

        Ok(())
    }

    /// Removes both keys. Missing ones are not an error — unregistering twice is harmless.
    pub fn unregister() -> Result<()> {
        let classes = CURRENT_USER
            .create("Software\\Classes")
            .context("opening HKCU\\Software\\Classes")?;
        // `remove_tree` rather than `remove_key`: the class has children, and a key with children
        // cannot be deleted on its own.
        let _ = classes.remove_tree(".kmpkg");
        let _ = classes.remove_tree(PROG_ID);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    /// Writes a MIME definition, a desktop entry and the icons, then asks the caches to notice.
    ///
    /// Three files and two commands, all under `~/.local/share`, which is the per-user half of the
    /// XDG data directories and needs no root.
    ///
    /// **The `.deb` does the same job a different way**, shipping the same three kinds of file into
    /// `/usr/share`, and the two coexist by design: a per-user entry wins over a system one for the
    /// user who wrote it, and both name the same icon theme entry and the same MIME type. What this
    /// exists for is the tarball, which installs nothing and is the shape somebody unpacks anywhere.
    pub fn register(exe: &Path) -> Result<()> {
        let share = data_home()?;

        // What the file *is*: an extension, a type name, and a human-readable description.
        let mime_dir = share.join("mime/packages");
        std::fs::create_dir_all(&mime_dir)
            .with_context(|| format!("making {}", mime_dir.display()))?;
        std::fs::write(mime_dir.join("karaokemachine-package.xml"), MIME_DEFINITION)
            .context("writing the MIME definition")?;

        // What opens it.
        let apps = share.join("applications");
        std::fs::create_dir_all(&apps).with_context(|| format!("making {}", apps.display()))?;
        std::fs::write(apps.join("karaokemachine.desktop"), desktop_entry_for(exe))
            .context("writing the desktop entry")?;

        // What the entry above points at. The `hicolor` theme is the fallback every desktop is
        // required to look in, so installing here needs no knowledge of which one is running.
        let icons = share.join("icons/hicolor");
        for &(name, pngs) in ICON_SETS {
            for &(size, bytes) in pngs {
                let dir = icons.join(format!("{size}x{size}/apps"));
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("making {}", dir.display()))?;
                let path = dir.join(format!("{name}.png"));
                std::fs::write(&path, bytes)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
        }

        refresh_caches(&share, &apps, &icons);

        Ok(())
    }

    /// Removes what `register` wrote, and refreshes the caches.
    pub fn unregister() -> Result<()> {
        let share = data_home()?;
        let apps = share.join("applications");

        let _ = std::fs::remove_file(share.join("mime/packages/karaokemachine-package.xml"));
        let _ = std::fs::remove_file(apps.join("karaokemachine.desktop"));

        // The icons too, or an uninstalled machine leaves its mark in every application menu that
        // reads the theme. The directories are left alone: `hicolor/48x48/apps` is shared with
        // everything else the user has installed, and removing one is not ours to do.
        let icons = share.join("icons/hicolor");
        for &(name, pngs) in ICON_SETS {
            for &(size, _) in pngs {
                let _ = std::fs::remove_file(icons.join(format!("{size}x{size}/apps/{name}.png")));
            }
        }

        refresh_caches(&share, &apps, &icons);
        Ok(())
    }

    /// The per-user data directory every one of these files goes under.
    ///
    /// **`XDG_DATA_HOME` first, and `~/.local/share` is its documented default rather than a second
    /// answer.** A relative value is ignored, which the specification requires and is not
    /// pedantry: a desktop reads the variable itself, so honoring a path it would refuse would put
    /// the entry somewhere nothing looks.
    fn data_home() -> Result<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .context("neither XDG_DATA_HOME nor HOME is set, so there is no per-user place to register in")
    }

    /// The shipped desktop entry with every `Exec=` line pointed at this executable.
    ///
    /// **`TryExec` as well as `Exec`, so a desktop that checks first hides an entry whose folder
    /// has been moved or deleted** rather than offering one that fails when it is pressed. It is
    /// inserted rather than shipped, because the packaged file has a `/usr/bin` symlink to name and
    /// nothing to check.
    ///
    /// **Every `Exec=` is rewritten and only the first gets a `TryExec`.** The file carries an
    /// action group as well as the entry itself, so a rewrite that stopped at the first line would
    /// leave the action naming a command that is only on the `PATH` of an installed machine — the
    /// one case a tarball unpacked anywhere is not. `TryExec` belongs to `[Desktop Entry]`, where
    /// the specification gives it a meaning, and hiding the entry hides its actions with it.
    ///
    /// The rest of each line is carried across untouched, so the `%f` that decides a double-click
    /// hands over one local path is stated once, in the file the `.deb` installs.
    pub(super) fn desktop_entry_for(exe: &Path) -> String {
        let exe = exe.to_string_lossy();
        let mut out = String::with_capacity(DESKTOP_ENTRY.len() + 2 * exe.len());
        let mut checked_it_is_there = false;
        for line in DESKTOP_ENTRY.lines() {
            match line.strip_prefix(PACKAGED_EXEC) {
                Some(arguments) => {
                    if !checked_it_is_there {
                        out.push_str(&format!("TryExec={exe}\n"));
                        checked_it_is_there = true;
                    }
                    out.push_str(&format!("Exec={}{arguments}\n", quoted(&exe)));
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// A path as an `Exec=` word, quoted where the specification says it has to be.
    ///
    /// **A tarball unpacked into a folder with a space in its name is the ordinary case this is
    /// for**, and an unquoted path there is a desktop entry that silently starts nothing. The
    /// reserved characters are the specification's; the four escaped inside the quotes are the ones
    /// it names, and the backslash is escaped first so the others' escapes survive.
    ///
    /// `TryExec` is deliberately not put through this: it is a single file name rather than a
    /// command line, and quotes there would be part of the name being looked for.
    fn quoted(exe: &str) -> String {
        const RESERVED: &[char] = &[
            ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#',
            '(', ')', '`',
        ];
        if !exe.contains(RESERVED) {
            return exe.to_owned();
        }
        let escaped = exe
            .replace('\\', r"\\")
            .replace('"', r#"\""#)
            .replace('`', r"\`")
            .replace('$', r"\$");
        format!("\"{escaped}\"")
    }

    /// Asks the three caches to notice what just changed.
    ///
    /// **Every one of them is advisory**: without them the entry works after the next login, and a
    /// container or a minimal desktop may have none of the commands installed.
    ///
    /// `gtk-update-icon-cache` gets flags where the others do not, and needs them: `-f` because a
    /// per-user theme has no `index.theme` and it otherwise refuses the directory outright, `-t` to
    /// accept that, and `-q` so a desktop that has no such cache says nothing on somebody's console.
    fn refresh_caches(share: &Path, apps: &Path, icons: &Path) {
        let mime = share.join("mime");
        let commands: [(&str, &[&str], &Path); 3] = [
            ("update-mime-database", &[], &mime),
            ("update-desktop-database", &[], apps),
            ("gtk-update-icon-cache", &["-qtf"], icons),
        ];
        for (command, flags, argument) in commands {
            match std::process::Command::new(command)
                .args(flags)
                .arg(argument)
                .status()
            {
                Ok(status) if status.success() => {}
                Ok(status) => tracing::debug!("{command} exited with {status}"),
                Err(error) => tracing::debug!("{command} is not installed: {error}"),
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use anyhow::bail;

    /// Asks LaunchServices to re-read the bundle this executable is inside.
    ///
    /// **There is nothing to write.** The document type is declared in the bundle's `Info.plist` —
    /// `CFBundleDocumentTypes` naming an exported UTI — and LaunchServices reads it when the `.app`
    /// is installed or first opened. So the only thing a command can usefully do is nudge it, which
    /// matters during development, when a bundle is rebuilt in place and the database still holds
    /// the previous one.
    ///
    /// Refuses when this is a bare executable rather than a bundled one, because there is genuinely
    /// nothing to register and saying so beats appearing to succeed.
    pub fn register(exe: &Path) -> Result<()> {
        let Some(bundle) = bundle_of(exe) else {
            bail!(
                "this is not inside a .app bundle, and on macOS the file type is declared by the \
                 bundle rather than by a command. Use Karaoke Machine.app -- the one the setup \
                 package put in /Applications, or the one staged beside this folder."
            );
        };
        let lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks\
                          /LaunchServices.framework/Support/lsregister";
        let status = std::process::Command::new(lsregister)
            .arg("-f")
            .arg(&bundle)
            .status()
            .context("running lsregister")?;
        if !status.success() {
            bail!("lsregister exited with {status}");
        }
        Ok(())
    }

    /// Nothing to undo: the declaration lives in the bundle, so removing the bundle removes it.
    pub fn unregister() -> Result<()> {
        bail!(
            "on macOS the file type belongs to the bundle. Move Karaoke Machine.app to the Trash to \
             remove it."
        )
    }

    /// The `.app` directory an executable sits inside, if it does.
    ///
    /// A bundled executable is at `Foo.app/Contents/MacOS/foo`, so the bundle is three levels up.
    pub(super) fn bundle_of(exe: &Path) -> Option<PathBuf> {
        let macos = exe.parent()?;
        if macos.file_name()? != "MacOS" {
            return None;
        }
        let contents = macos.parent()?;
        if contents.file_name()? != "Contents" {
            return None;
        }
        let bundle = contents.parent()?;
        bundle
            .extension()
            .is_some_and(|ext| ext == "app")
            .then(|| bundle.to_path_buf())
    }
}

/// Platforms with no file-type association to write at all, which means **Android and iOS**.
///
/// **This arm is what makes the crate compile there, and it is unreachable in the build that needs
/// it.** `--register` arrives through [`crate::cli`], and neither mobile platform goes near `cli` —
/// SDL calls `run_on_phone` directly, and a `SDL_main` is handed no argument vector to carry a flag
/// in. The alternative was a `#[cfg]` around `mod register` *and* both `cli` arms that reach it,
/// which compiles out a front end already dead on those platforms, in three places instead of one.
///
/// **Both platforms have the association and neither has anything to write, which is why the arm
/// refuses rather than doing something.** A bundle declares its own types and a manifest declares
/// its own filters: `ports/machine/ios/project.yml` carries the `CFBundleDocumentTypes` and
/// `UTExportedTypeDeclarations` pair, and `ports/machine/android/app/src/main/AndroidManifest.xml`
/// carries two `ACTION_VIEW` filters, so *Open with* reaches the machine on both with nothing
/// written at run time. The macOS arm above can at least nudge LaunchServices with `lsregister`
/// while a bundle is rebuilt in place; there is no such command on a phone and no development loop
/// that would want one.
///
/// Its absence was a real build break rather than a theoretical one: `mod platform` had arms for
/// Windows, Linux and macOS, so `armv7-linux-androideabi` matched none and `platform::register`
/// failed to resolve. **`task check` cannot see this** — it builds for the host — so the first
/// thing to notice was `task build:android`.
///
/// It refuses rather than succeeding quietly, which is the judgment `display::FILE_MANAGER` makes
/// for `F10`: say there is nothing here rather than report a success that associated nothing.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;
    use anyhow::bail;

    /// Refuses: there is no per-user file-type database on this platform to write into.
    pub fn register(_exe: &Path) -> Result<()> {
        bail!(
            "this platform has no file-type association to register. The application declares the \
             type itself, so opening a package already reaches it."
        )
    }

    /// Refuses, for the same reason: nothing was ever written.
    pub fn unregister() -> Result<()> {
        bail!("this platform has no file-type association to remove")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded path is this executable's own, which is what makes an unpacked tarball work.
    #[test]
    fn the_path_written_down_is_this_executable() {
        let exe = executable().expect("this test is running from somewhere");
        assert!(
            exe.is_absolute(),
            "a relative path would not survive a move"
        );
        assert!(exe.exists());
    }

    /// The console twin registers the windowed executable beside it, not itself.
    ///
    /// The whole of the bug this exists to prevent: `--set-password` and `--show-paths` live on the
    /// console program, so that is the one somebody has in their hand when they run `--register` —
    /// and a `.kmpkg` associated with it would open a console window from a file manager, on the one
    /// platform where having a windowed twin at all is the point.
    #[test]
    fn the_console_twin_hands_the_registration_to_the_windowed_one() {
        let folder = std::env::temp_dir().join(format!("km-register-twin-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a scratch folder");

        let extension = if cfg!(windows) { ".exe" } else { "" };
        let windowed = folder.join(format!("karaokemachine{extension}"));
        let console = folder.join(format!("karaokemachine-console{extension}"));
        std::fs::write(&console, b"a stand-in; this never gets run").expect("write");

        // Alone in a folder somebody copied one file into, the twin is all there is.
        assert_eq!(windowed_twin_of(&console), None);

        std::fs::write(&windowed, b"a stand-in; this never gets run").expect("write");
        assert_eq!(windowed_twin_of(&console), Some(windowed.clone()));
        // ...and the windowed one is never redirected anywhere, which would be a loop.
        assert_eq!(windowed_twin_of(&windowed), None);

        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The type the two shipped files have to agree on, which is the only thing that checks it.
    ///
    /// Here rather than beside the constants above, because nothing in the program spells it any
    /// more: the desktop entry claims it and the definition declares it, both as shipped bytes.
    #[cfg(target_os = "linux")]
    const MIME_TYPE: &str = "application/x-km-package";

    /// The two shipped files name one type and one icon, and this is what holds them to it.
    ///
    /// They are separate documents doing separate jobs — one says this program opens the type, the
    /// other says the type exists — so nothing but a test makes them agree. A disagreement is
    /// silent in the worst way: every file is installed, every command succeeds, and a `.kmpkg`
    /// stays a nameless icon that opens nothing.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_entry_and_the_definition_name_one_type_and_one_icon() {
        assert!(
            DESKTOP_ENTRY.contains(&format!("MimeType={MIME_TYPE};")),
            "the desktop entry must claim the type the definition declares"
        );
        assert!(
            MIME_DEFINITION.contains(&format!(r#"type="{MIME_TYPE}""#)),
            "the definition must declare the type the desktop entry claims"
        );
        assert!(
            DESKTOP_ENTRY.contains(&format!("\nIcon={ICON_NAME}\n")),
            "the desktop entry must name the icon theme entry the PNGs are written under"
        );
        // **The action's own, and it is a separate assertion because the line above cannot see it.**
        // `Icon=karaokemachine-stream` starts with `Icon=karaokemachine`, so an action that lost its
        // icon key entirely would leave that check passing on the entry's — and the failure is a
        // stream entry in a menu wearing the mark of the thing beside it.
        assert!(
            DESKTOP_ENTRY.contains(&format!("\nIcon={STREAM_ICON_NAME}\n")),
            "the stream action must name the badged mark"
        );
        assert!(
            ICON_SETS.iter().any(|&(name, _)| name == STREAM_ICON_NAME),
            "the badged mark must be installed under the name the action asks for"
        );
        assert!(
            MIME_DEFINITION.contains(&format!(r#"icon name="{ICON_NAME}""#)),
            "a package file should wear the mark of the thing that opens it"
        );
        assert!(
            DESKTOP_ENTRY.contains(PACKAGED_EXEC),
            "the line --register rewrites has to be the line the shipped file carries"
        );
    }

    /// The definition is XML a parser will accept, and its prose is the only part that can stop it.
    ///
    /// A `--` inside a comment body is not XML, and `update-mime-database` rejects the whole
    /// document over it rather than the comment. The type is then never added, so a `.kmpkg` keeps
    /// no name, no icon and no default application, and the desktop entry's `MimeType=` line is
    /// never read — the failure the file's own comment is there to warn about.
    ///
    /// **Nothing between here and an install on a real machine says so.** The `.deb`'s trigger
    /// prints the parse error and carries on, `postinst` sends its own run to `/dev/null`, and the
    /// verifiers ask `grep` questions a malformed document answers.
    ///
    /// Ungated, unlike its neighbours, because the file is edited on every platform and only one of
    /// them parses it.
    #[test]
    fn the_definition_is_xml_a_parser_will_accept() {
        const DEFINITION: &str = include_str!("../linux/karaokemachine-package.xml");

        let mut rest = DEFINITION;
        while let Some(open) = rest.find("<!--") {
            let body = &rest[open + 4..];
            let Some(close) = body.find("-->") else {
                panic!("a comment is opened and never closed");
            };
            assert!(
                !body[..close].contains("--"),
                "a comment body may not hold `--`, and this one does: {}",
                &body[..close]
            );
            rest = &body[close + 3..];
        }
    }

    /// `--register` writes the shipped entry with this executable's path in it.
    ///
    /// Both halves matter. The path is what makes the entry work from a folder somebody unpacked
    /// anywhere, and the rest of the file is what a hand-written copy here used to lose —
    /// `StartupWMClass` above all, which is what ties the running window back to this entry's icon.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_entry_written_here_is_the_shipped_one_pointed_at_this_executable() {
        let written = platform::desktop_entry_for(Path::new("/tunes/km/karaokemachine"));

        assert!(written.contains("\nExec=/tunes/km/karaokemachine %f\n"));
        assert!(written.contains("\nTryExec=/tunes/km/karaokemachine\n"));
        assert!(
            !written.contains("Exec=karaokemachine %f"),
            "the packaged command name must not survive the rewrite"
        );
        // Carried across rather than restated, which is the point of embedding the file.
        assert!(written.contains("StartupWMClass=karaokemachine"));
        assert!(written.contains("Keywords=karaoke;midi;lyrics;singing;"));
        assert!(written.contains(&format!("MimeType={MIME_TYPE};")));
    }

    /// Both commands in the file are pointed at this executable, and exactly one `TryExec` is added.
    ///
    /// **The action's `Exec=` is the one a rewrite stopping at the first match would leave behind**,
    /// and what it would leave is the packaged name — which resolves only on an installed machine,
    /// the one case a tarball unpacked anywhere is not. A second `TryExec` is the other direction:
    /// the specification gives that key a meaning in `[Desktop Entry]` and none in an action group.
    #[cfg(target_os = "linux")]
    #[test]
    fn every_command_in_the_entry_is_rewritten_and_only_the_entry_is_checked() {
        let written = platform::desktop_entry_for(Path::new("/tunes/km/karaokemachine"));

        assert!(written.contains("\nExec=/tunes/km/karaokemachine %f\n"));
        assert!(written.contains("\nExec=/tunes/km/karaokemachine --stream\n"));
        // **Key lines only, because the comments are carried across untouched** and one of them
        // names the prefix this rewrite matches on. A comment naming it is what the file is for; a
        // key still naming it is the fault.
        assert!(
            !written
                .lines()
                .filter(|line| !line.starts_with('#'))
                .any(|line| line.starts_with(PACKAGED_EXEC)),
            "the packaged command name survived the rewrite on a key line"
        );
        assert_eq!(
            written.matches("TryExec=").count(),
            1,
            "`TryExec` belongs to [Desktop Entry] and to nothing below it"
        );

        // And it is above the action group, which is the half the count cannot see.
        let entry = written
            .split_once("[Desktop Action")
            .expect("the file declares an action group")
            .0;
        assert!(entry.contains("TryExec=/tunes/km/karaokemachine\n"));
    }

    /// A folder with a space in its name is a working entry, not a silent one.
    ///
    /// `~/My Programs/karaokemachine` is an ordinary place to unpack a tarball, and an unquoted
    /// path there is a desktop entry that starts nothing and says nothing. `TryExec` is a file name
    /// rather than a command line and is deliberately left bare.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_path_with_a_space_is_quoted_in_exec_and_bare_in_tryexec() {
        let written = platform::desktop_entry_for(Path::new("/tunes/My Programs/km"));

        assert!(written.contains("\nExec=\"/tunes/My Programs/km\" %f\n"));
        assert!(written.contains("\nTryExec=/tunes/My Programs/km\n"));
    }

    /// A bare executable is not inside a bundle, and macOS says so rather than pretending.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bare_executable_is_not_a_bundle() {
        assert_eq!(
            platform::bundle_of(Path::new("/usr/local/bin/karaokemachine")),
            None
        );
        assert_eq!(
            platform::bundle_of(Path::new(
                "/Applications/Karaoke Machine.app/Contents/MacOS/km"
            )),
            Some(PathBuf::from("/Applications/Karaoke Machine.app"))
        );
    }
}
