# On a Samsung television, Apps2Samsung signs it

**A Samsung television installs an app from outside its store only when the app is signed for that
television.** So the `.tpk` on the release page is not signed for yours, and the installer does the
last step. It takes about ten minutes the first time, and a computer on the same network as the
television.

**Which televisions:** it has run on a 2025 Samsung television on Tizen 10. A 2024 one, on Tizen 8,
is likely to work and has not been tried. Older ones are not supported.

You need a free Samsung account, and **[Apps2Samsung](https://github.com/Apps2Samsung/Apps2Samsung)**
on Windows, macOS or Linux.

1. Download `karaokemachine-<version>-tizen.tpk`.
2. On the television, open **Apps**, then type **1 2 3 4 5** with the remote's number pad. Switch
   **Developer mode** on, enter the computer's address as **Host PC IP**, and restart the television.
   Apps2Samsung shows the address to type.
3. In Apps2Samsung, choose the television, then install the `.tpk` as a local file. The first time,
   it opens a Samsung sign-in page to make a certificate for your television.
4. On the television, start **Karaoke Machine** from **Apps**. It shows its name for a second, then
   the idle screen.

**Songs go in from a browser.** The idle screen gives the machine's address, with the PIN to sign in
with. Open that address on a computer or a phone, go to the owner's page, and send a song package
there. See [Getting songs in](songs.md).

Four limits come from the television rather than from the application:

- **Developer mode may lapse after about a week.** Whether the app still starts after that has not
  been measured yet. If it stops starting, repeat step 2 and step 3.
- **The remote has no number keys**, so song numbers are typed on the machine's own number pad, on
  the idle screen. The remote's arrows move around it, and its Play/Pause, Stop and Next keys work
  during a song.
- **The picture is 1080p**, because the television gives an app nothing larger. Video songs in 4K or
  in HEVC are not supported.
- **Installing a new version over an old one does not always replace it.** If the new version
  behaves like the old one, remove Karaoke Machine on the television and install it again. Removing
  it also removes its songs and settings.
