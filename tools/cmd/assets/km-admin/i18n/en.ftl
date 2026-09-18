# KaraokeMachine Admin's own pages, in English.
#
# **Plain application language**, which is the rule this file is headed with because every catalog in
# the project is: `What a user reads is written in plain application language`. The reader here is an
# owner setting a machine up on a desktop, so the register is the operator's — plainer than the
# singer's remote and no more technical than the errand needs.
#
# **This program's own half only.** *This machine*, Songs, Pictures and Sound are `km-admin-pages`'
# markup and are drawn from that crate's catalog. What is here is the front door, the picture
# searching and the bank fetching — the half `A fourth program, rather than a fourth tab on the
# owner's page` keeps off the machine.
#
# **The data stays as it arrives.** A bank's *what it is like* note and its license come out of
# `km-banks`; a provider's terms and a photograph's credit come from the provider. Those are values,
# like a song's title on the remote, and translating them is not this file's job.

## The front door ------------------------------------------------------------
#
# Which machine, and the password for it. The two are one form because they are one errand: nothing
# on the tabs behind this page can write to a machine until it has been told which one and been let
# in. The password box is labelled from a key of this file's own even though the shared catalog has
# one for the same box, because `Catalogs live beside the words they translate` puts a key where its
# markup is; what keeps the two saying the same thing is the rule, not the file.

door-heading = Which machine?
door-password = The machine's password
door-explained = Everything this program sends goes to the machine you pick here. It has to be switched on, so that it can check the password.
door-chosen-unnamed = The machine you last chose
door-last-used = last used
door-typed = Another address
door-address-example = 192.168.1.50
door-looking = Looking for machines on this network…
door-none-found = No machines answered. Type an address above, or look again once the machine is on.
door-look-again = Look again
door-this-one = this one
door-use = Use this machine
door-saved-for = This computer has the password for { $machine }.
door-saved-here = This computer has this machine's password.
door-already = This program is already logged in to this machine.
door-retype = Type a different password
door-retype-again = Type the password again
door-remember = Remember this password on this computer
door-remember-open = On this computer the file is protected by your user profile and nothing more.
door-forget = Forget it
door-where = The password is a six-digit code on the machine's own screen until somebody changes it.

# The picker by which somebody sets what language **this program** is in, on the card below the
# door.
#
# **The heading says which of two languages it means**, because the Machine tab has the other one:
# that pane sets what the television draws in, and the likeliest misreading of either is that they
# are one thing. The languages inside the control name themselves rather than being translated —
# see `Locale::endonym`.
#
# The note says what the choice reaches and what it does not: these pages, on this computer, and no
# machine anywhere.
door-locale-heading = What language this program is in
door-locale-save = Save
door-locale-note = This is about the pages you are reading, on this computer. What the machine's own screen says is Screen language, on the This machine tab.
door-locale-changed = These pages are now in this language.
door-needs-an-address = Type an address, or pick a machine.
door-no-machine = That address could not be used.
door-password-refused = The machine did not accept that password.
door-needs-a-password = This machine's password is needed before this program can be used with it.
door-unreachable = That machine did not answer. Check that it is switched on and that the address is right.
send-needs-password = This machine asks for its password before anything can be sent to it. Type it here, then send again.
send-needs-machine = Pick a machine before sending anything to one.
bank-unknown = There is no such bank in the list.
bank-not-here = That bank is not in this program's folder.
bank-remove-failed = { $bank } could not be removed: { $why }
pack-unknown = There is no such pack in this program's folder.
pack-remove-failed = { $pack } could not be removed: { $why }
keys-not-remembered = The key was not written down: { $why }
keys-not-forgotten = The keys were not deleted: { $why }
job-busy-search = A search is already running.
job-busy-fetch = Something is already being fetched.
search-needs-terms = There is nothing to search for yet.
search-needs-key = { $provider } needs a key of your own before it will answer.

## What a job is doing -------------------------------------------------------
#
# Keys rather than words because a phase is written down where no request is in reach — see
# `job::phase`. Four of the six are `km-wallpaper-pack`'s own phases, mapped at this program's
# boundary by `job::phase_key`.

phase-starting = starting
phase-searching = searching
phase-downloading = downloading
phase-measuring = measuring
phase-building = building
phase-sending = sending

job-stopping = stopping
job-stop = Stop

## The bank table ------------------------------------------------------------

sound-heading = Sound
sound-lead = A sound bank supplies the instruments a MIDI song is played with. Download one here and send it to the machine.
sound-note = The note on each row comes from listening to that bank. The license shown is what the bank's own file states. Downloads come from each publisher's own address; this project mirrors nothing. If your machine has its own internet connection, it can download banks itself.

banks-no-answer = The machine did not answer, so this list cannot show which banks it already has. You can still download banks; they are saved on this computer.

column-bank = Bank
column-size = Size
column-what-its-like = What it is like
column-license = License

bank-recommended = Recommended
bank-shortlist = shortlist
bank-here = on this computer
bank-on-the-machine = on the machine
bank-by-hand-from-site = by hand, from its own site
bank-by-hand-only = by hand only
bank-too-large = too large to send — the machine can fetch this one itself

action-get = Get
action-send = Send
action-send-again = Send again
action-remove = Remove

# Composed in Rust: the name is the machine's own, so it arrives as a field. See `words::COMPOSED`.
bank-remove-confirm = Remove { $bank } from this program's folder? The machine keeps whatever it already has.

## Packs on this computer ----------------------------------------------------

packs-heading = Packs on this computer
packs-empty = No packs yet. A finished search saves its pack here, and it stays until you remove it.
packs-note = Each pack is a zip file you can send to any number of machines. Removing one deletes it from this computer only; a machine keeps what it has already been sent.

column-pack = Pack
column-pictures = Pictures
column-built = Built

pack-remove-confirm = Remove { $pack } from this program's folder? A machine you have already sent it to keeps its copy.

## Finding pictures ----------------------------------------------------------

pictures-heading = Pictures
pictures-lead = Find background pictures with enough contrast for lyrics to stay readable, and send them to the machine. Contrast is measured in the part of the screen the lyrics occupy, against the colour they are drawn in and the dimming the machine applies.

providers-heading = Where to look
provider-no-account = no account needed
provider-key-set = key set
provider-needs-key = needs your key
provider-get-key = Get one at

key-field = A key for the provider you chose
key-placeholder = leave blank to keep the one already set
key-remember = Remember it on this machine
key-remember-unix = Written to this program's own folder, readable only by you.
# Two whole sentences rather than a sentence with a hole in it. The second is the emphasised one,
# and it is the `<strong>` in the markup — a message split around its own emphasis would force
# English word order onto every other language.
key-remember-windows-lead = Written to this program's own folder.
key-remember-windows-warning = Windows has no per-file owner-only setting this program can set, so it is protected by your profile folder and nothing more.

pack-name-field = Call the pack
pack-name-hint = Goes into the zip's name, which is also what the machine will call it. Lowercase letters, digits and dashes; anything else is dropped, and what is left is shown back here.
pack-name-needed = Give this pack a name before searching. It goes into the file's name, which is how you will tell two packs apart.

terms-field = What to search for, one per line
pages-field = Pages per term
count-field = Pictures in the pack
contrast-field = Contrast to reach
contrast-hint = 7.0 is WCAG AAA, which is what the machine's own pack was built to.
ask-width-field = Ask for at least (px wide)
ask-width-hint = What the provider is asked for, by its own description of the photograph.
keep-width-field = Keep at least (px wide)
# `too_small_on_disk` stays inside the sentence and unstyled: it is the exact word the Rejected list
# above shows, so naming it is the point, and pulling it out into its own `<code>` would have split
# the sentence around it.
keep-width-hint = The width the downloaded file must actually have, which is often less than the provider reported. If results are all rejected as too_small_on_disk, lower this.

action-save = Save
action-forget-keys = Forget every remembered key

search-heading = Search
search-note = Only the search step downloads anything. Measuring and building reuse what has already been downloaded, so you can change the settings above and run again without searching again.
search-kept = The finished pack is saved on this computer. Send it to a machine from the list below.
action-search = Search, measure and build

review-heading = What came of it
# Composed in Rust: two counts, and a plural is arithmetic.
review-verdict = { $chosen } of { $looked_at } { $looked_at ->
        [one] picture
       *[other] pictures
    } came through the gate.
review-may-pass-on = this pack may be passed on
review-for-this-machine = for the machine that built it
review-rejected = Rejected:

# Composed in Rust: the credit is the provider's, and arrives as a field.
picture-alt = a candidate wallpaper by { $author }
picture-contrast = contrast
