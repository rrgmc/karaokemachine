# Getting songs in

The machine starts with no songs, because you supply your own. Songs arrive in **packages**. A
package is one `.kmpkg` file that carries its songs with their queue numbers, titles, artists and
analysis. Videos and MP3+G pairs travel inside it.

To try the machine first, download the carol package from the
[release page](https://github.com/rrgmc/karaokemachine/releases). To make a package from a folder of
your own files, see [Getting a corpus into shape](packaging.md).

## Install a package

Pick whichever of these is nearest to hand. Each one puts the file into the packages folder, so the
songs survive a restart.

- **Double-click the package.** The machine installs it and says so.
- **Drag the file onto the machine's window.** It shows `installing …`, then how many songs went in.
- **Copy the file into the packages folder yourself.** `F10` opens the folder, and `Ctrl+F10` reads
  it again, so the songs play without a restart.
- **Add it from a browser**, on the **Songs** tab of [the setup page](setup.md).
- **Send it through [the API](api.md).**

The double-click needs the machine registered for `.kmpkg` files. The Windows installer offers to do
that, and macOS does it when it installs the app. On Linux and for the portable Windows folder, run
`karaokemachine --register` once. Neither needs administrator rights.

A rebuilt package of the same name replaces the old one.

## Remove a package

**The packages folder says which packages are installed.** Take a package out of the folder, and the
machine uninstalls it at the next start or the next `Ctrl+F10`. Removing a package on the **Songs**
tab deletes its `.kmpkg`.

## The carol package

**One package is available to download: sixteen Christmas carols.** Every one is public domain.
`CREDITS.md` beside the pack names every source.

No install carries it, and no other songs come with the machine. A karaoke MIDI is rarely free to
distribute, because the tune, the arrangement and the words each have an owner.

## How songs are numbered

A singer picks a song by its number, so the numbers matter once you print a list.

**A package holds at most 999 songs**, and the machine holds up to a thousand packages. The limit is
there to encourage a curated volume.

**Each package sits in a block of a thousand numbers**, called its bank. A song's number is
`bank × 1000 + slot`, where the slot is the number the package gave the song. Two packages that both
number a song 500 therefore do not clash: one is 3500, the other 611500.

**A package's bank comes from its id, so its numbers are the same on every machine.** A printed song
list therefore travels with the file. If another package already holds that bank, the package takes
the next one.

## Give a package shorter numbers

Put the volume you sing from most into a low block, and its songs take four digits instead of six.
Choose any block from 1 to 9999, because bank 0 belongs to the machine. The **Songs** tab at
`http://127.0.0.1:8177/admin/` has a box for it.

**Do it once, when the package goes in.** A change renumbers every song in that package, so a printed
list becomes wrong. The machine refuses the change while a song of the package plays or waits in the
queue.
