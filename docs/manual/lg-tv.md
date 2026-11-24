# On an LG television, unofficially, through Developer Mode

**The `.ipk` is an unofficial build, for people comfortable with developer tools.** An LG television
installs an app from outside LG's store only in Developer Mode, and **when Developer Mode lapses the
television removes the app, with its songs and settings.** For a television that should simply keep
working, use an Android or Google TV box with the Android APK instead.

**Which televisions:** it has run on a 2020 LG television on webOS 5. Other webOS versions have not
been tried.

You need a free LG developer account, a computer on the same network as the television, and LG's
webOS command-line tools, which need [Node.js](https://nodejs.org/).

1. Download `karaokemachine-<version>-webos.ipk`.
2. On the television, install **Developer Mode** from the LG Content Store and sign in with your LG
   developer account. Switch **Dev Mode Status** on and let the television restart, then open
   Developer Mode again and switch **Key Server** on. Leave it open: it shows the television's
   address and a passphrase.
3. On the computer, install the tools with `npm install -g @webos-tools/cli`. Add the television
   with `ares-setup-device`, giving it a name such as `tv`, the address Developer Mode shows, port
   `9922` and user `prisoner`. Then run `ares-novacom --device tv --getkey` and type the passphrase.
4. Install the app with `ares-install --device tv karaokemachine-<version>-webos.ipk`.
5. On the television, start **Karaoke Machine** from the app list. It shows the idle screen.

**Songs go in from a browser.** The idle screen gives the machine's address, with the PIN to sign in
with. Open that address on a computer or a phone, go to the owner's page, and send a song package
there. See [Getting songs in](songs.md).

The limits come from the television rather than from the application:

- **Developer Mode lasts 1,000 hours, about six weeks.** Press **Extend** in the Developer Mode app,
  with the television online, to start the count again. If it lapses, the app and its songs are
  removed, and you install again from step 2. Ten restarts without a network connection also switch
  it off.
- **Installing a new version over an old one keeps the songs and settings**, and so does switching
  the television off and on.
- **There is little room for songs.** Apps share about 700 MB. MIDI songs take almost nothing, but
  an MP3+G song takes 6 to 8 MB and a video song far more, so a few hundred of those fill it. A large
  sound bank takes room as well as memory: the television plays a 275 MB bank, but only just.
- **The picture is 1080p.** Video songs in H.264 play at 720p and 1080p.
- **The remote's Back key goes back**, and its number keys type song numbers.
