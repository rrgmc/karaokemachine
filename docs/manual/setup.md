# Setting it up from a browser

The owner of the machine sets it up on one page, from any browser on the network. Open the machine's
address and add `/admin`. The remote also links to the page, at the foot of its Setup tab.

The singer's remote is a different page. It offers only search, queue, and the controls a song
allows, and nothing on it can delete songs.

## Sign in with the password on the screen

**The machine shows the password on its own screen.** It generates a six-digit PIN at first start and
shows it beside its address. Anyone in the room can read it, and nobody outside can.

Change it on **This machine**, under **Access**. Every tab links there until you do.

## The five tabs

| Tab | What you do there |
|---|---|
| **This machine** | See where to reach it. Then five panes: **General**, **Access**, **Singers and queue**, **Playback** and **Developer**. |
| **Songs** | See the installed packages, how many songs each holds, and the thousand its numbers sit in. Add or remove a package. Choose whether the machine takes linked packages. |
| **Pictures** | See the pictures in the rotation and the one that shows, and add more. Your own pictures replace the ones that came with the machine. |
| **Sound** | Choose the sound bank that plays and the audio output, and add a `.sf2`. |
| **Problems** | See packages the machine could not load, and anything else that is wrong, each with the control that fixes it. The tab shows a count. |

**This machine holds its settings in five panes, each named for its topic.**

| Pane | What you set there |
|---|---|
| **General** | The machine's name, the language of its screen, whether a Windows or macOS window shows its Menu, and restart or power off |
| **Access** | The password, who may do what, and signing every phone and browser out |
| **Singers and queue** | The queue order, what the machine asks of a singer, and the night after a restart |
| **Playback** | The announcement before each song, demo mode, whether a song ends when the rest is silent, and whether a recorded song changes key and tempo |
| **Developer** | Debugging, the console and the performance overlay |

**The Songs tab says whether the machine takes linked packages.** The **Linked packages** card has
one box, and it starts off. [Getting songs in](songs.md#linked-packages) says what a linked package
is, and how to set the folder its songs play from.

## Decide what a guest's phone can do

**The Access pane says who may do what.** A phone with no code gets the room level, and you
choose that level:

| Room level | A phone with no code can |
|---|---|
| Watch only | Browse the songs and see the queue and what is playing. |
| Ask for songs | Also ask for a song. The song takes its place by the queue order on the **Singers** card. |
| Ask for songs, add a song out of turn, and change key and tempo | Also add a song with no singer, which takes no turn. Also change the key, tempo, volume and guide melody. |
| All of that, skip and play now | Also skip, play a song now, use the other transport buttons, and move or remove anybody's song. This is the default. |

**At the default level every guest's phone is a full remote.** That suits a home, where everybody
in the room is trusted. Only the admin password changes the machine's settings. At a party of
strangers, choose *Ask for songs*: then a guest cannot skip, or change the key or the tempo of
another singer's song. [Running it at a venue](venue.md) gives the rest.

**Two codes raise one phone at a time above the room level.** Both are on the **Who may do what**
card of the Access pane. The first adds a song with no singer and changes key and tempo. The second
also skips, and it opens the host's tab. Give each to whoever should have it. They type it on the
remote's Setup tab, which then says what that phone can do.

Clearing or changing a code takes that level back from every phone that used it.

**The Singers and queue pane says what the machine asks of a singer.** Its **Singers** card has four answers:

| Setting | What it does |
|---|---|
| Queue order | **In the order asked**, the default, puts every song at the end of the queue as it is asked for. **Singers take turns** keeps one song for each singer in the queue, and a singer's next song waits until the one before starts. Choose turns where strangers share the machine. |
| Every singer gives a name | A phone must type a name before its first song. Turn it on where a host calls each singer by name. |
| Singers on one phone | The most singers one phone may add. Leave it empty for no limit, which suits a home that passes one phone around. |
| Songs one singer may have waiting | When singers take turns, the most songs a singer may have waiting behind the one in the queue. It starts at 10. |

**A change of queue order applies at once.** A change to **In the order asked** puts every waiting
song into the queue, in the order the songs were asked for.

**The same pane says whether the machine keeps the night through a restart.** The
**The evening after a restart** card has one box, and it starts off. With the box ticked, the
machine starts with the queue, the singers and the waiting songs it had. It then plays nothing
until somebody presses play. [Running a public night](venue.md#after-a-power-cut-or-a-restart) says
what the host does next.
