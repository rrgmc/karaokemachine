# Privacy

**No program sends telemetry, crash reports or usage data, and none checks for updates.** A crash
report stays in a file on the computer. A program contacts a host on the internet only for what
the list below names.

- **The Windows setup program** downloads Microsoft's WebView2 installer when you choose a tool that
  needs WebView2 and the computer does not have it.
- **The machine downloads an instrument bank** when you ask it to. It also downloads one at its first
  start when you left that checkbox ticked in the setup program.
- **KM Admin downloads an instrument bank** when you click to fetch one.
- **KM Admin and `km-wallpaper-pack` search Openverse, Pixabay or Pexels** when you ask for pictures.
  The search terms go to that service, and with them the API key you gave it, if any. The pictures
  then download from the addresses the service returns.

A download sends a plain request and nothing that identifies you. **On your own network**, the
machine answers the HTTP API and announces itself over mDNS, and the remotes and tools reach it there.
A *YouTube* link in a remote or the package builder opens your browser only when you click it.
