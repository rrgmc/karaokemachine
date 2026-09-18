//! Telling the operating system what a `.kmbuild` file is.
//!
//! `--register` associates the extension with *this* executable, wherever it currently is, and
//! `--unregister` takes it back off. Recording the current path rather than an installed one is what
//! makes it work for a portable folder somebody unzipped anywhere, which is the only shape this tool
//! ships in.
//!
//! **Per-user, never machine-wide.** On Windows that means `HKEY_CURRENT_USER`, which needs no
//! elevation; on Linux, `~/.local/share`, which needs no root. A tool that asked for administrator
//! rights to make double-clicking work would be a worse trade than typing the path.
//!
//! **macOS does it differently and does not come through here at all.** A document type is declared
//! in a bundle's `Info.plist` — `CFBundleDocumentTypes` plus an exported UTI — and LaunchServices
//! picks it up when the `.app` is placed or launched. There is nothing for a command to write, so
//! `--register` there only nudges LaunchServices at a bundle that already declares everything.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The MIME type `.kmbuild` files are given on Linux.
///
/// `x-` because it is not registered with IANA and is not going to be. It names this product rather
/// than the format, since the format is "a SQLite database this tool wrote".
#[cfg(target_os = "linux")]
const MIME_TYPE: &str = "application/x-km-package-builder";

/// The icon-theme name the desktop entry points at, and the basename each PNG is installed under.
///
/// One string for the file in `icon/`, the `Icon=` line and the installed path, so a rename cannot
/// leave two of the three agreeing.
#[cfg(target_os = "linux")]
const ICON_NAME: &str = "km-package-builder";

/// This tool's icon at each size a desktop might ask for, compiled in.
///
/// **Embedded rather than staged beside the executable**, because `tools/dist/cmd.sh` stages a
/// bare exe and a README and nothing else — a folder somebody unzipped anywhere is the only shape
/// this tool ships in, so an icon file next to it is a file a copy leaves behind. 68 KB, and the
/// alternative is what stood here until now.
///
/// **Until now nothing installed these at all**, while the desktop entry below has always said
/// `Icon=km-package-builder`. A theme name with no file behind it is not an error anywhere: the
/// entry simply drew whatever generic icon the desktop keeps for an application it cannot picture,
/// and did so silently, which is why it survived. The blue mark is what made it worth finding.
#[cfg(target_os = "linux")]
const ICON_PNGS: &[(u32, &[u8])] = &[
    (
        16,
        include_bytes!("../../../../icon/km-package-builder-16.png"),
    ),
    (
        32,
        include_bytes!("../../../../icon/km-package-builder-32.png"),
    ),
    (
        48,
        include_bytes!("../../../../icon/km-package-builder-48.png"),
    ),
    (
        64,
        include_bytes!("../../../../icon/km-package-builder-64.png"),
    ),
    (
        128,
        include_bytes!("../../../../icon/km-package-builder-128.png"),
    ),
    (
        256,
        include_bytes!("../../../../icon/km-package-builder-256.png"),
    ),
];

/// What the console twin is called, next to the windowed executable it belongs to. See
/// [`windowed_twin_of`].
const CONSOLE_SUFFIX: &str = "-console";

/// The windowed executable beside this one, when this one is the console twin.
///
/// **Registering has to name the program a double-click should reach, and that is never this one.**
/// `--help` and `--version` live on `km-package-builder-console`, so that is the executable somebody
/// is most likely to be typing when they run `--register` — and recording it would associate every
/// corpus in a file manager with the program that deliberately has no window. Nobody would find that
/// out until they double-clicked a `.kmbuild` and got a browser tab.
///
/// `None` for the windowed executable itself, and for a console twin whose partner is not there —
/// somebody who copied one file out of the staged folder gets themselves registered, which works and
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

/// Where the executable to register is, tidied.
///
/// The path that gets written down.
///
/// **`current_exe` does not resolve symlinks everywhere.** On
/// Apple platforms `std` is `_NSGetExecutablePath` and nothing else — no `realpath` — so through a
/// symlink this records the symlink. That is harmless here and is why it is a note rather than a
/// fix: the macOS arm below refuses on anything that is not inside a bundle, and a symlink into
/// `Contents/MacOS` is exactly one of the shapes it refuses. Windows and Linux, where a written-down
/// path is what `--register` is for, do canonicalise.
fn executable() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding this executable")?;
    let exe = windowed_twin_of(&exe).unwrap_or(exe);
    Ok(crate::model::tidy(&exe))
}

/// Associates `.kmbuild` with this executable.
pub fn register() -> Result<()> {
    let exe = executable()?;
    platform::register(&exe)?;
    km_console::say(format!(
        "  registered  .kmbuild opens with {}",
        exe.display()
    ));
    km_console::say("  Double-click a corpus's .kmbuild file to open it.");
    Ok(())
}

/// Takes the association back off.
pub fn unregister() -> Result<()> {
    platform::unregister()?;
    km_console::say("  unregistered  .kmbuild is no longer associated with this tool");
    Ok(())
}

#[cfg(windows)]
mod platform {
    use super::*;

    use windows_registry::CURRENT_USER;

    /// The class name the extension points at.
    ///
    /// A `Vendor.Product` name rather than something like `kmbuildfile`, which is the convention and
    /// which also makes it obvious in `regedit` who put it there.
    const PROG_ID: &str = "KaraokeMachine.PackageBuilder";

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
            .create(".kmbuild")
            .context("creating the .kmbuild key")?;
        extension
            .set_string("", PROG_ID)
            .context("naming the class for .kmbuild")?;

        // ...and the class says what it is called, what it looks like, and how to open it.
        let class = classes.create(PROG_ID).context("creating the class key")?;
        class
            .set_string("", "Karaoke corpus")
            .context("naming the class")?;

        // The icon comes out of the executable's own resource, which `build.rs` already attaches --
        // so a `.kmbuild` in Explorer gets this tool's blue mark with nothing extra to ship or to
        // lose, and follows `build.rs` automatically if the mark ever changes again.
        let icon = class
            .create("DefaultIcon")
            .context("creating DefaultIcon")?;
        icon.set_string("", format!("\"{}\",0", exe.display()))
            .context("setting the document icon")?;

        let command = class
            .create("shell\\open\\command")
            .context("creating the open command")?;
        // `"%1"` quoted, because a corpus path with a space in it is the normal case rather than the
        // exotic one, and an unquoted one arrives split across several arguments.
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
        let _ = classes.remove_tree(".kmbuild");
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
    /// XDG data directories and needs no root. This is the same shape the Linux tarball's own
    /// `install.sh` already uses for the machine itself.
    pub fn register(exe: &Path) -> Result<()> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set, so there is no per-user place to register in")?;
        let share = home.join(".local/share");

        // What the file *is*: an extension, a type name, and a human-readable description.
        let mime_dir = share.join("mime/packages");
        std::fs::create_dir_all(&mime_dir)
            .with_context(|| format!("making {}", mime_dir.display()))?;
        std::fs::write(
            mime_dir.join("km-package-builder.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="{MIME_TYPE}">
    <comment>Karaoke corpus</comment>
    <glob pattern="*.kmbuild"/>
  </mime-type>
</mime-info>
"#
            ),
        )
        .context("writing the MIME definition")?;

        // What opens it. `%f` is a single local file path, which is what a double-click delivers;
        // `%U` would offer URLs this tool cannot read.
        let apps = share.join("applications");
        std::fs::create_dir_all(&apps).with_context(|| format!("making {}", apps.display()))?;
        std::fs::write(
            apps.join("km-package-builder.desktop"),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name={name}\n\
                 Comment=Browse karaoke files and curate them into packages\n\
                 Exec={exe} %f\n\
                 TryExec={exe}\n\
                 Icon={ICON_NAME}\n\
                 Terminal=false\n\
                 Categories=AudioVideo;Audio;Utility;\n\
                 MimeType={MIME_TYPE};\n",
                // A launcher's nine characters, so the short name — see `APP_NAME_SHORT`.
                name = crate::APP_NAME_SHORT,
                exe = exe.display()
            ),
        )
        .context("writing the desktop entry")?;

        // What the entry above points at. The `hicolor` theme is the fallback every desktop is
        // required to look in, so installing here needs no knowledge of which one is running.
        let icons = share.join("icons/hicolor");
        for &(size, bytes) in ICON_PNGS {
            let dir = icons.join(format!("{size}x{size}/apps"));
            std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
            let path = dir.join(format!("{ICON_NAME}.png"));
            std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        }

        refresh_caches(&share, &apps, &icons);

        Ok(())
    }

    /// Removes what `register` wrote, and refreshes the caches.
    pub fn unregister() -> Result<()> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("HOME is not set")?;
        let share = home.join(".local/share");
        let apps = share.join("applications");

        let _ = std::fs::remove_file(share.join("mime/packages/km-package-builder.xml"));
        let _ = std::fs::remove_file(apps.join("km-package-builder.desktop"));

        // The icons too, or an uninstalled tool leaves its mark in every application menu that
        // reads the theme. The directories are left alone: `hicolor/48x48/apps` is shared with
        // everything else the user has installed, and removing one is not ours to do.
        let icons = share.join("icons/hicolor");
        for &(size, _) in ICON_PNGS {
            let _ = std::fs::remove_file(icons.join(format!("{size}x{size}/apps/{ICON_NAME}.png")));
        }

        refresh_caches(&share, &apps, &icons);
        Ok(())
    }

    /// Asks the three caches to notice what just changed.
    ///
    /// **Every one of them is advisory**: without them the entry works after the next login, and a
    /// container or a minimal desktop may have none of the commands installed. Shared by `register`
    /// and `unregister` because a half-removed entry is as confusing as a missing one, and the two
    /// had already drifted — the removal path never refreshed the icon cache because it never wrote
    /// icons to begin with.
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
    /// matters during development, when a bundle is rebuilt in place and the database still holds the
    /// previous one.
    ///
    /// Refuses when this is a bare executable rather than a bundled one, because there is genuinely
    /// nothing to register and saying so beats appearing to succeed.
    pub fn register(exe: &Path) -> Result<()> {
        let Some(bundle) = bundle_of(exe) else {
            // The bundle on disk wears the short name, so this reads the short constant rather
            // than spelling a folder's name out where a rename would not find it.
            bail!(
                "this is not inside a .app bundle, and on macOS the file type is declared by the \
                 bundle rather than by a command. Use {name}.app -- the one the setup package put \
                 in /Applications, or the one staged beside this folder.",
                name = crate::APP_NAME_SHORT
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
            "on macOS the file type belongs to the bundle. Move {name}.app to the Trash to \
             remove it.",
            name = crate::APP_NAME_SHORT
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Scratch;

    /// The recorded path is this executable's own, which is what makes a portable folder work.
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
    /// The whole of the bug this exists to prevent: `--help` lives on the console program, so that is
    /// the one somebody has in their hand when they run `--register` — and a `.kmbuild` associated
    /// with it would open a browser tab from a file manager, which is the opposite of what
    /// double-clicking a corpus is for.
    #[test]
    fn the_console_twin_hands_the_registration_to_the_windowed_one() {
        let scratch = Scratch::new("register-twin");
        let folder = scratch.0.clone();

        let extension = if cfg!(windows) { ".exe" } else { "" };
        let windowed = folder.join(format!("km-package-builder{extension}"));
        let console = folder.join(format!("km-package-builder-console{extension}"));
        std::fs::write(&console, b"a stand-in; this never gets run").expect("write");

        // Alone in a folder somebody copied one file into, the twin is all there is.
        assert_eq!(windowed_twin_of(&console), None);

        std::fs::write(&windowed, b"a stand-in; this never gets run").expect("write");
        assert_eq!(windowed_twin_of(&console), Some(windowed.clone()));
        // ...and the windowed one is never redirected anywhere, which would be a loop.
        assert_eq!(windowed_twin_of(&windowed), None);
    }

    /// A bare executable is not inside a bundle, and macOS says so rather than pretending.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bare_executable_is_not_a_bundle() {
        assert_eq!(
            platform::bundle_of(Path::new("/usr/local/bin/km-package-builder")),
            None
        );
        assert_eq!(
            platform::bundle_of(Path::new("/Applications/Foo.app/Contents/MacOS/foo")),
            Some(PathBuf::from("/Applications/Foo.app"))
        );
    }
}
