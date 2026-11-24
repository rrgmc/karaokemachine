# On an LG television, unofficially, through Developer Mode

**The `.ipk` is an unofficial build, for people comfortable with developer tools.** An LG television
installs an app from outside LG's store only in Developer Mode, and **when Developer Mode lapses the
television removes the app, with its songs and settings.** For a television that should simply keep
working, use an Android or Google TV box with the Android APK instead.

**Which televisions:** it has run on a 2020 LG television on webOS 5. Other webOS versions have not
been tried.

You need a free LG developer account, a computer on the same network as the television, and
[**webOS Dev Manager**](https://github.com/webosbrew/dev-manager-desktop/releases/latest), a free
desktop program from the webOS Homebrew project for Windows, macOS and Linux. It is not LG's, and
not part of the Karaoke Machine.

1. Download `karaokemachine-<version>-webos.ipk`, and install webOS Dev Manager on the computer.
2. On the television, install **Developer Mode** from the LG Content Store and sign in with your LG
   developer account. Switch **Dev Mode Status** on and let the television restart, then open
   Developer Mode again and switch **Key Server** on. Leave it open: it shows the television's
   address and a passphrase.
3. In webOS Dev Manager, add the television with the address and the passphrase Developer Mode
   shows.
4. Open **Apps** and install `karaokemachine-<version>-webos.ipk` from the file.
5. On the television, start **Karaoke Machine** from the app list. It shows the idle screen.

**LG's own command-line tools work too**, for anyone who already uses them:
`npm install -g @webos-tools/cli`, then `ares-setup-device` with the address, port `9922` and user
`prisoner`, `ares-novacom --device tv --getkey` with the passphrase, and
`ares-install --device tv karaokemachine-<version>-webos.ipk`.

**Songs go in from a browser.** The idle screen gives the machine's address, with the PIN to sign in
with. Open that address on a computer or a phone, go to the owner's page, and send a song package
there. See [Getting songs in](songs.md).

The limits come from the television rather than from the application:

- **Developer Mode lasts 1,000 hours, about six weeks.** Press **Extend** in the Developer Mode app,
  with the television online, to start the count again. webOS Dev Manager shows the time left on the
  television's information page, renews it with **Renew session**, and **Renew automatically**
  writes a script a computer can run on a schedule to keep renewing it. If it lapses, the app and its songs are
  removed, and you install again from step 2. Ten restarts without a network connection also switch
  it off.
- **Installing a new version over an old one keeps the songs and settings**, and so does switching
  the television off and on.
- **There is little room for songs.** Apps share about 700 MB. MIDI songs take almost nothing, but
  an MP3+G song takes 6 to 8 MB and a video song far more, so a few hundred of those fill it. A large
  sound bank takes room as well as memory: the television plays a 275 MB bank, but only just.
- **The picture is 1080p.** Video songs in H.264 play at 720p and 1080p.
- **The remote's Back key goes back**, and its number keys type song numbers.
