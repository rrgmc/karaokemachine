# On Debian, it is also an appliance — if you ask

**The `.deb` includes a systemd service, switched off by default.** Enable it, and the computer
becomes an appliance. It starts when the power returns, and it draws from a bare virtual terminal
with no desktop installed.

```sh
sudo systemctl enable --now karaokemachine
```

`sudo systemctl disable --now karaokemachine` turns it off again.

- **Do it on a machine with no desktop**, not on your laptop. The service takes over `tty1`. A login
  screen such as GDM, LightDM or SDDM holds the graphics device, and the television stays black.
- **It runs as a system user, `karaoke`**, so its songs, settings and catalog live under
  `/var/lib/karaoke`. Packages go in `/var/lib/karaoke/.local/share/karaokemachine/packages`.
  `sudo systemctl status karaokemachine` says whether it runs, and `sudo journalctl -t karaokemachine`
  says what happened.
