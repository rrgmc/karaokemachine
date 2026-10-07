# On Debian, it is also an appliance — if you ask

An appliance is a box that does one thing. Plug it into a television, switch it on, and it is a
karaoke machine, with no desktop and nothing to log in to. The `.deb` can turn a Debian computer into
one.

## Turn it on

**The `.deb` includes a systemd service, switched off by default.** Enable it, and the computer
starts the machine whenever the power returns.

```sh
sudo systemctl enable --now karaokemachine
```

`sudo systemctl disable --now karaokemachine` turns it off again.

## Choose the right computer

**Do it on a machine with no desktop**, not on your laptop. The service takes over `tty1`, and it
draws from that bare virtual terminal. A login screen such as GDM, LightDM or SDDM holds the graphics
device, and the television stays black.

## Where it keeps things

**It runs as a system user, `karaoke`**, so its songs, settings and catalog live under
`/var/lib/karaoke`.

| To | Use |
|---|---|
| Add a package | `/var/lib/karaoke/.local/share/karaokemachine/packages` |
| See whether it runs | `sudo systemctl status karaokemachine` |
| See what happened | `sudo journalctl -t karaokemachine` |

`Ctrl+Q` on the appliance switches the box off, as its power button does.
