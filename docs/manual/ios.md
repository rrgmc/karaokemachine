# On an iPhone or an iPad, you sign it yourself

**iOS installs an application only under a signature, and Apple issues none that a stranger can give
you.** So both `.ipa` files say `unsigned` in the name, and you do the last step. It takes about five
minutes and a computer, once per app.

You need an Apple ID, and the one on the phone is fine. You also need a free signing tool:
**[Sideloadly](https://sideloadly.io)** on macOS or Windows, or **[AltStore](https://altstore.io)**.

1. Download `karaokemachine-<version>-ios-unsigned.ipa`, or
   `km-remote-<version>-ios-unsigned.ipa` for the remote alone.
2. Connect the phone or the iPad to the computer and unlock it.
3. Open Sideloadly, drag the `.ipa` onto it, enter your Apple ID and press **Start**. It signs the
   app for your devices and installs it.
4. On the device, open **Settings → General → VPN & Device Management**, tap your Apple ID under
   *Developer App*, and tap **Trust**. The app does not open until you do.
5. Start it once with the network available. iOS verifies the certificate on that first run.

Four limits come from Apple rather than from the application:

- **A free Apple ID signs for seven days.** After that the app does not open until you repeat step 3.
  Your songs, settings and catalog stay on the device. A paid developer account signs for a year, and
  AltStore renews in the background.
- **A free Apple ID signs three applications at a time.** The machine and the remote are two of them.
- **The machine does not announce itself on the network**, because Apple grants the multicast
  entitlement only after review. The iOS remote finds it anyway. The Android remote and a browser need
  the address from the idle screen.
- **An idle machine stops answering in the background**, because iOS suspends an app that plays no
  audio. A song that plays keeps playing with the screen off.

**Most people want the remote.** It is small, the last two limits do not apply to it, and it is what a
guest holds.
