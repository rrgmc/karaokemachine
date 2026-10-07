# Removing it

**Your songs, settings and catalog stay where they are**, and the uninstaller names their folders.

- **Windows**: uninstall as you do anything else. The remote alone is its own entry, **KM Remote**.
- **macOS**: open `/usr/local/karaokemachine` and double-click **Uninstall KaraokeMachine**. It lists
  what it removes, asks, and then asks for your password. From a terminal, run
  `sudo /usr/local/karaokemachine/uninstall.sh`, with `--dry-run` for the list alone. The remote alone
  has `/usr/local/km-remote` and **Uninstall KM Remote**.
- **Debian**: `sudo apt remove karaokemachine`, which stops and disables the appliance service first.
  `/var/lib/karaoke` stays even on a `purge`, and `sudo userdel -r karaoke` removes it.
- **Any Linux**: delete the folder. If you ran its `install.sh` for a menu entry,
  `./install.sh --uninstall` removes that entry.

Removing the machine leaves the remote alone, and removing the remote leaves the machine alone.
