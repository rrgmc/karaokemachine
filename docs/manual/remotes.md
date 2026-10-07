# The remotes

A phone is the machine's remote. A singer searches the catalog on it, queues a song, and sees what
plays. The machine has two remotes: a page it serves itself, and a separate program that works with
the machine switched off.

## The remote the machine serves

**The machine serves a remote at its own address**, and any phone on the network opens it. Nobody
installs an app.

**To open it, point a phone's camera at the QR code on the idle screen.** The screen also shows the
address, so nobody types an IP address. Mid-song, `I` shows the address and the code. `F11` opens the
same page in the browser of the computer the machine runs on. See [The keyboard](keyboard.md).

<p align="center">
<img src="../images/screen-idle-connect.png" width="90%"
     alt="The idle screen: a partly typed song number in blue, a disc in the corner counting the
songs waiting, and a panel giving the machine's address on the network beside a QR code">
<br><sub><b>The idle screen.</b> Point a phone at the code, and the remote opens.</sub>
</p>

The remote has a page for each thing a singer does: find a song, follow the song that plays, and see
who is next.

<table>
<tr>
<td width="33%"><img src="../images/remote-browse.png" alt="The singer's remote on a phone: mode
buttons for songs, artists and a printed book, a search box, a language filter, and a list of songs
with artist, duration and number, each with a button to look it up on YouTube and a button to add it
to the queue"></td>
<td width="33%"><img src="../images/remote-now.png" alt="The remote's now-playing page: the song,
the singer it was queued for, a progress bar, and steppers for key and tempo, a music volume slider
and a guide-melody toggle"></td>
<td width="33%"><img src="../images/remote-queue.png" alt="The remote's queue page: the song
playing across the top with its transport folded away behind a toggle, then the queued songs by
position, title, artist and the singer who asked for each, their move and remove buttons folded away
behind a second toggle"></td>
</tr>
<tr>
<td><sub><b>Search and queue</b> from any phone. No app to install.</sub></td>
<td><sub><b>The song as it plays</b>, with its key, tempo, guide melody and music volume.</sub></td>
<td><sub><b>Who is up next</b>, changeable by anyone with the skip level. The controls stay
folded until asked for.</sub></td>
</tr>
</table>

**The owner decides how much a guest's phone can do.** Out of the box, anybody in the room can queue
a song. See [Setting it up from a browser](setup.md#decide-what-a-guests-phone-can-do).

## The offline remote

**The offline remote is a separate program**, `km-remote`. It keeps its own copy of a machine's
catalog, so browsing, searching and favorites work **with the machine switched off**. You can choose
your songs before the machine is on.

It finds a machine on the network and remembers it. It also adds two things the served remote does
not have: favorites, and an A–Z picker.

<p align="center">
<img src="../images/remote-offline.png" width="33%"
     alt="The offline remote showing its song list, with a favorites mode, an A to Z picker, and a
star beside each song for filing it as a favorite">
<br><sub><b>The offline remote</b>, with its favorites mode and its A–Z picker.</sub>
</p>

It has its own download on every platform it runs on. See [Installing](installing.md).
