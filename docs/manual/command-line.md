# The command line

```sh
karaokemachine --show-paths              # settings, catalog, packages folder, assets
karaokemachine --set-password hunter2    # change the admin password (or the /admin page, or the API)
karaokemachine --reset-password          # back to a fresh PIN, shown on the machine's screen
karaokemachine --reset-sessions          # sign every phone and browser out at once
karaokemachine --set-name "Living Room"  # what phones call this machine on the network
                                         # (or set it at http://<the machine>/admin)
karaokemachine vol1.kmpkg                # install a package (what a double-click does)
karaokemachine --register                # make .kmpkg files open with this machine
karaokemachine --unregister              # ...and take that back off
karaokemachine --list-audio-devices      # output devices and their stable ids
karaokemachine --song-book ./songbook.pdf   # every installed song, as a PDF to print
karaokemachine --headless                # no window: API + engine + catalog
karaokemachine --stream                  # no window: the screen goes to http://<the machine>/watch/
karaokemachine --fullscreen              # fill the screen this run, whatever settings say
karaokemachine --windowed                # ...and open in a window instead
karaokemachine -v                        # say more; -vv for everything
karaokemachine --frame-stats             # fps, frame times and decode, once a second
karaokemachine --log-file                # also write this run's log to a file
```

**`--log-file` is for a run that went wrong.** A double-click opens no console, so the log has
nowhere else to go. It writes one file per run into a `logs` folder beside the catalog, and keeps the
ten newest.

**`--fullscreen` and `--windowed` apply to one run and write nothing down.** An installed machine
fills the screen, and one you built yourself opens in a window.
