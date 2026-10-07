# Getting songs in

**Songs arrive in packages.** A `.kmpkg` is one file that carries its songs, queue numbers, titles,
artists and analysis. Videos and MP3+G pairs travel inside it.

**Double-click the package**, and the machine installs it and says so. The Windows installer offers
to set this up, and macOS does it when it installs the app. On Linux and for the portable Windows
folder, run `karaokemachine --register` once. Neither needs administrator rights.

**Or drag the file onto the machine's window.** It shows `installing …`, then how many songs went in.
A rebuilt package of the same name replaces the old one.

Either way the file goes into the packages folder, so it survives a restart. You can also copy the
file into that folder yourself, or send it through [the API](api.md).

**The packages folder says which packages are installed.** Take a package out, and the machine
uninstalls it at the next start or the next `Ctrl+F10`. Removing a package on the **Songs** page
deletes its `.kmpkg`.

**One package is available to download: sixteen Christmas carols.** Every one is public domain, and
no install carries it, because you supply your own songs. A karaoke MIDI is rarely free to
distribute, because the tune, the arrangement and the words each have an owner. `CREDITS.md` beside
the pack names every source.

**A package holds at most 999 songs.** The limit is there to encourage a curated volume. The machine
holds up to a thousand packages.

**A song's number is `bank × 1000 + slot`.** The slot is the number the package gave the song, and
the bank is the block of a thousand the package sits in. Two packages that both number a song 500 do
not clash: one is 3500, the other 611500.

**A package's bank comes from its id, so its numbers are the same on every machine.** A printed song
list therefore travels with the file. If another package holds that bank, the package takes the next
one.

**You can choose a block**, from 1 to 9999, because bank 0 belongs to the machine. Put the volume you
sing from most into a low block, and its songs take four digits instead of six. The **Songs** page at
`http://127.0.0.1:8177/admin/` has a box for it.

A change renumbers every song in that package, so **a printed list becomes wrong**. The machine
refuses while a song of the package plays or waits in the queue. Do it once, when the package goes
in.

To make a package from a folder of your own files, see
[Getting a corpus into shape](packaging.md).
