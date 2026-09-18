# Learning Rust by reading this repository

*A reading path for somebody who has written C++ for a decade and Rust for no time at all.*

**Not a plan.** `docs/decisions/` says what the product must do and why, and `docs/ARCHITECTURE.md`
says how it is being built. This says how to *read* what was built, and it is a commitment about
neither — it is reference documentation in the sense `CLAUDE.md` means, describing what is true
rather than what will be. The `file.rs:LINE` anchors are the one part of it that rots silently: every one
was verified against the source when this was written, and code moves.

---

## What this is, and what it assumes

You know what a hash map is. You know why memory safety matters. You have written enough C++ that
`Vec<Entry>` reads correctly on sight and RAII is not a technique but a habit. None of the usual
introductory material is aimed at you, and most of it wastes the first third of its length on
things you settled twenty years ago.

What you do not have is the handful of places where **C++ intuition is actively wrong about Rust**
— not absent, wrong, which is worse, because a wrong model produces confident misreadings rather
than confusion. There are about fifteen of these. This document walks you to each one through code
you already own the intent of.

That last part is the whole reason to use this repository as the text rather than a tutorial. The
usual failure of introductory Rust is that the borrow checker never actually *forces* anything: the
examples are too small to have a design, so ownership looks like ceremony. This codebase is 233,539
lines across 316 files and contains the forcing — several times, in places where the comment beside
the code says which design was abandoned and why. You will not have to guess.

**How to read it.** Six stages, in order. Each is: the C++ concept this is about to complicate, an
ordered list of things to read, what to notice in them, and the corrections. Each stage is an
evening or two. Nothing here asks you to write code — that comes after, and it comes easily once
the reading is done.

**Where it stops.** At *fluency*: able to read and confidently change most of this workspace. It
deliberately stops before async, threads, `unsafe` and FFI. Those are all here, they are the most
interesting code in the repository, and they are Appendix B rather than Stage 7 — because meeting
`Pin` before you have internalised `&mut` is how people bounce off Rust.

**One warning about the source.** This codebase has an unusually high comment-to-code ratio, and
the comments argue design trade-offs rather than restating the code. **The comments are the point.**
Skimming to the code will lose most of the value, and in several places below the comment is the
lesson and the code is the illustration.

---

## Stage 0 — Before reading anything

Two reasons this is not skippable. First, reading Rust without type-on-hover is much harder than
reading C++ without an IDE: inference means the types frequently are not written down, and
`let entries = …` tells you nothing on paper that `rust-analyzer` will tell you instantly. Second,
a green build now means that when something fails later it is you and not the machine.

```sh
rustup show                 # this repo pins an exact rustc + rustfmt + clippy in rust-toolchain.toml
cargo km-test               # the alias for the test command; see .cargo/config.toml
cargo km-lint               # clippy with -D warnings
```

`cargo test` on its own is **not** enough here — `km-song` and `km-api` both keep their test
fixtures behind a `testing` feature, so the aliases exist to spell that out. `--all-features` is
deliberately not used anywhere in this workspace and would fail on Linux; the reason is in
`CLAUDE.md`.

Then set up navigation:

- **rust-analyzer**, with inlay type hints on. Non-negotiable.
- `cargo doc --open -p km-songcode`. Worth doing once for the surprise: the workspace sets
  `missing_docs = "warn"`, so every public item carries prose, and `cargo doc` renders the design
  essays into a browsable manual. There is no C++ equivalent of this being *routine*.

Finally, skim [`docs/decisions/README.md`](decisions/README.md) once. Source comments cite decisions
by name ("see the `Unsafe code, once` decision in docs/decisions/"), and it is much easier to follow
a citation when you have already seen the index it comes from.

**One fact that will save you an afternoon of confusion with online material:** this workspace is
**edition 2024**, `rust-version = "1.98.1"`. A great deal of what is written about Rust on the
internet is edition 2015 or 2018 and differs on module paths, on `dyn`, on `impl Trait`, and on
whether `unsafe` is spelled as a block or an attribute. When a tutorial disagrees with this
codebase about syntax, the codebase is newer.

---

## Stage 1 — Reading Rust: one crate, end to end

> **The C++ sentence.** You can already read a header file. This stage is about learning that in
> Rust the header and the implementation are the same file, that the `impl` block is not part of
> the type declaration, and that the tests are in there too.

### Read

**`crates/song/km-songcode/src/lib.rs` — all 362 lines.** The entire crate, one file, two dependencies.

This is the best first read in the repository and you should not substitute anything else for it.
In one sitting it contains: `//!` (module doc) against `///` (item doc), `const`,
`#[derive(...)]`, a `thiserror` error enum, a newtype with a private field, `impl` blocks, `&str`
against `String` and the `to_owned()` that converts, `Result` and `?`, `impl Display`,
`impl FromStr` with an associated type, `const fn`, hand-written `Serialize`/`Deserialize`, and a
`#[cfg(test)] mod tests` with `use super::*`.

It also happens to be a genuinely good piece of design, so you are not reading a toy.

### Notice

- **`impl` blocks are separate from the type.** `SongCode` is declared at `:115` and its methods
  live at `:117-161`, with three more blocks at `:163`, `:169` and `:202` — and there could be five
  such blocks in five files. This is not a style choice; it is what makes it possible to implement a
  trait for a type you did not define.
- **`SongCode` is a newtype with one private field** (`:115`), and `new` at `:123`, `in_bank` at
  `:155` and `from_str` at `:180` are the only ways to make one. Rust has no `private:` section; the
  default *is* private, and `pub` is what you write to open something up. The default direction is
  the opposite of a C++ `struct`.
- **`:197-198`** — `// Safe by the bound just checked.` above a `number as u32`, where `number` is a
  `u64` that `:192` has already filtered against `MAX_NUMBER`. Note what this is: an invariant
  established a few lines up and *relied on* here, with the reliance written down. You do this in
  C++ too; the difference is that the cast is narrowing and the comment is what says why it cannot
  lose anything.
- **`:93-107`** — the error type is an `enum`, not a class hierarchy, and each variant carries
  exactly the data its message needs. `#[error("…")]` generates the `Display` impl. There is no
  base class and there is nothing to catch. Read `NumberTooLarge`'s doc at `:101-106`: it carries
  the text as typed rather than a `u32` *because the value may not fit one*, which is a design
  decision the type system forced into the open.
- **`:180-199`** — `from_str`. Read the doc block at `:172-179` and the comment at `:185-188`; both
  are arguments about product behavior, not about Rust. This is the register the whole codebase is
  written in.
- **`:291-303`** — the test asserting that `MAX_BANK`, `MAX_SLOT` and `MAX_NUMBER` are one statement
  written three ways. That is types-as-proofs at the level of constants: three numbers that must
  agree, with a test that fails when somebody widens one of them alone, rather than a comment asking
  the next reader to notice.
- **`:109-113`** — the `SongCode` doc, which says the ordering is the number's own and so is what
  `u32` already does. Worth reading beside the `Ord` derive at `:114`: the property is free *because*
  of how the value is represented, and the doc says so instead of leaving you to work it out.

### What your C++ instinct gets wrong here

- **`String` and `&str` are not `std::string` and `const std::string&`.** They are closer to
  `std::string` and `std::string_view`, and the conversion has a name (`to_owned()`, `to_string()`)
  because it allocates. When you see `to_owned()` in this file, an allocation is being paid for
  deliberately.
- **`#[derive(Debug, Clone, Copy, …)]` is not `= default`.** It is code generation from a trait,
  and the generated code is real code you can reason about — the `Ord` derive at `:114` is
  field-by-field in declaration order, and with one field that is the `u32`'s own ordering, which is
  exactly why the representation was chosen.
- **`Copy` is not a copy constructor.** It means "this type is safe to duplicate with a memcpy and
  I want that to happen implicitly." `SongCode` is `Copy`; `String` can never be. There is no user
  code involved and no way to write any.
- **The tests being in the same file is normal**, not a shortcut. `#[cfg(test)]` means the module
  is compiled only under `cargo test`, so it costs the shipped binary nothing.

---

## Stage 2 — Ownership and borrowing

> **The C++ sentence.** You know RAII, you know `const&` and `&&`, and you have spent years
> deciding by hand who owns what. Rust takes that decision out of the comments and puts it in the
> type system, where the compiler checks it. Most of what follows is familiar; one thing is not,
> and it is `&mut`.

This is the longest stage and the one to take slowly.

### Read, in order

**1. `crates/machine/km-queue/src/queue.rs:83-165` — read the signature column first.**

Before any function body. Just the receivers:

```
len(&self)          is_empty(&self)     entries(&self)      peek(&self)
add(&mut self)      remove(&mut self)   pop(&mut self)      move_to(&mut self)   clear(&mut self)
```

This looks *exactly* like C++ const and non-const member functions, and it is about 80% right.
The remaining 20% is the entire stage: `&self` means "shared, and there may be many"; `&mut self`
means **"exclusive, and there is exactly one"**. Not "mutable". Exclusive. C++ has no such thing —
`T&` guarantees you nothing about who else holds one — and this is the single largest difference
between the two languages.

**2. `crates/machine/km-queue/src/queue.rs:129-132` — the cleanest borrow-checker fingerprint in the repo.**

```rust
let index = self.entries.iter().position(|entry| entry.id == id)?;
self.entries.remove(index)
```

You would have written this with an iterator: find it, erase it. You cannot, because the
`&QueueEntry` that `iter()` yields borrows `self`, and `remove` needs `&mut self` — and those
cannot coexist. So the reference is converted to an index, which borrows nothing, and the borrow
ends. The same shape appears again at `:142-154` in `move_to`, this time with a genuine
bug-avoidance comment at `:149-150`.

Recognize this shape. Once you know it, a great deal of Rust that looks gratuitously indirect
stops looking that way.

**3. `crates/song/km-songbook/src/arrange.rs:73-96` — the ownership lesson.**

```rust
pub fn arrange(mut entries: Vec<Entry>, last: &str) -> Vec<BookSection>
```

A vector **by value**, mutated in place, and a *different* vector returned. Your C++ reflex says
this copies, so take `std::vector<Entry>&` and mutate through it — or take `&&` and be careful.
Neither is necessary. The `Vec` is moved in, and a move is a memcpy of three words with no
destructor call at the source. Passing by value here is free, and it is also *better*: the
signature says the caller is giving the vector up, which the reference version could not say.

The `mut` in `mut entries` is not part of the type. It is a statement about the local binding, and
callers neither see it nor care.

Also in these 24 lines: `sections.last_mut()` returning `Option<&mut BookSection>` (`:87`), a
`match` with a guard (`:88`), `#[must_use]` (`:72`), and the derived `Ord` on `SortKey` being
lexicographic in field-declaration order, which is why `artist_missing` is the first field
(`:26-34`).

**4. `crates/playback/km-audio/src/player.rs:203-212` — why `Option::take` exists.**

```rust
pub fn retire(&mut self) -> Option<Retired> {
    match self.program.take() {
```

You cannot move a field out of `&mut self` — you only borrowed it, and moving out would leave a
hole in something you do not own. So `take()` swaps `None` in and hands you the owned value, and
the struct is left in a valid state at every instant.

The deeper point: **Rust has no moved-from-but-usable object.** In C++ a moved-from `std::string`
is in a "valid but unspecified state" and reading it is legal, which is a rule you keep in your
head. Here the compiler statically forbids the second read. `take()` is what you write when you
genuinely need the "leave it empty" behavior, and it makes that intent explicit rather than
implicit.

**5. `crates/playback/km-display/src/keypad.rs:162-170` — `focus: Option<usize>`.**

An index (`:169`), not a reference, into the `keys: Vec<Key>` (`:163`) that the *same struct* owns.
This is not laziness. **Self-referential structs are not expressible in safe Rust** — the struct
would have to name its own lifetime — and the standard answer is to store an index. When you see an
index where you expected a pointer, this is usually why.

**6. `crates/machine/karaokemachine/src/machine.rs:1755-1787` — borrows visibly shaping a function body.**

`:1766` does `loaded.song().map(Arc::clone)` to get an *owned* handle out from under a lock guard.
`:1773-1774` carries the comment "Built while the borrow of `loaded` is still live, then the guard is
dropped before publishing". `:1784` is a bare `drop(state);`.

That last line is worth stopping on. `drop()` here is not destruction-for-side-effects, which is
what it would be in C++. **It ends a borrow.** The lock guard's scope would otherwise run to the
end of the function, and the publish on the next line would deadlock or block. `drop` is the tool
for saying "the borrow ends here", and you will see it used that way constantly.

**7. Sidebar: a `macro_rules!` reached for because of the borrow checker.**

There is none in the workspace to point at, so here is the whole of one — a helper that pushed one
plain-text argument onto an argv being assembled out of a mixture of `&str` literals and owned
`OsString`s made from paths:

```rust
macro_rules! flag {
    ($args:ident, $value:expr) => {
        $args.push(OsString::from($value))
    };
}
```

One screen, and a macro rather than a closure *because* a closure would have held a mutable borrow
of the vector and locked out every direct `push` interleaved between the calls. A
borrow-checker-driven reason to reach for a macro is a shape you would never predict from C++,
where a lambda capturing by reference costs you nothing at compile time and everything at runtime.

### Lifetimes, lightly

Read far more often than written. Two anchors, and then leave it alone until the compiler makes
you care.

**`crates/song/km-song/src/karaoke.rs:36`** — `pub struct MetaText<'a> { … pub bytes: &'a [u8] }`. A
`Copy` view into the MIDI file's buffer, so parsing allocates nothing. This is `string_view` with
the dangling problem solved: the annotation says "this cannot outlive the buffer it points into",
and the compiler enforces it.

**`crates/song/km-kmpkg/src/lib.rs:571-600`** — `EntryWindow`, whose doc line at `:573` reads, in
bold, *"The point of it is the lifetime."* Read the whole comment. `zip`'s own reader borrows the archive, so
it can never be `'static` and can never move to a decoder thread; this type learns the byte range,
drops the archive, keeps the file handle, and becomes `Read + Seek + Send + Sync + 'static`.

That is the anchor for the thing C++ programmers most reliably get wrong: **`'static` here is a
*bound*, not the lifetime of a reference.** `T: 'static` means "contains no borrows", which is
what a value must satisfy to be moved to another thread. It has nothing to do with static storage
duration, and reading it that way will confuse you for a month.

### What your C++ instinct gets wrong here

- **`&mut` is exclusive, not merely non-const.** Everything else in this stage follows from it.
- **A move is destructive and statically enforced.** There is no moved-from state to reason about,
  because there is no second read.
- **There are no move constructors, no copy constructors, no assignment operators.** Not "they are
  generated for you" — they do not exist as a concept. A move is always a memcpy and never runs
  user code. This deletes an entire chapter of C++ that you will not miss.
- **Passing by value is often the right answer**, and the signature is documentation. Reaching for
  `&` by reflex will produce worse Rust than the naive version.
- **`drop(x)` ends a borrow.** It is a scope tool.
- **`'static` is usually a bound meaning "owns everything it contains".**

---

## Stage 3 — Enums, `Option`, `Result`

> **The C++ sentence.** You have `enum class`, you have `std::variant`, you have `std::optional`,
> and you have exceptions. Rust has one mechanism where C++ has four, it is the same mechanism you
> already know as a tagged union, and the difference is that the compiler checks you handled every
> case.

### Read

**`crates/machine/km-api/src/error.rs:19-69`, then the three matches at `:104-116`, `:118-132`, `:134-…`.**

The richest enum here: tuple variants (`NotFound(String)`) and named-field variants
(`Conflict { code, message }`) in one type. Read the three matches *together* — they are the same
enum answered three ways, and between them they show exhaustiveness, `..` rest patterns, and
or-patterns (`Self::NotFound(_) | Self::UnknownEndpoint(_)`).

Notice that adding a variant to this enum breaks all three functions at compile time, by name.
That is the property you are buying.

**`crates/machine/km-queue/src/transport.rs:8-27`** — `Transport`, a four-state machine, `#[default]` on a
variant, and `matches!` at `:25`. Note `pub fn is_advancing(self)` at `:24` takes `self` **by
value**. Alarming to a C++ eye; free in fact, because `Transport` is `Copy` and one byte wide.

**`crates/playback/km-audio/src/player.rs:50-61`** — `Retired { Song(Arc<Song>), Track(Box<TrackPlayer>) }`.
`Box` and `Arc` side by side in one enum with the doc saying why each. Also `enum Program` at `:67`,
which is *private* — a type used only inside the module, which C++ makes awkward and Rust does not.

**`crates/machine/karaokemachine/src/machine.rs:245-284`** — `SongPreviewLookup` and `CatalogCounts`. Both are
"three outcomes, not an `Option`", and the docs argue why collapsing two of them into `None` would
be a bug. This is the best illustration in the repository of what sum types buy you over
`bool` / `optional` / error codes: the third state is *"the library is locked, ask again"*, and in
a design without it that becomes a lie. The `try_lock` that produces one is at `:2363`, the other
at `:2488`, and the exhaustive match is in a different module —
`crates/machine/karaokemachine/src/display.rs:3260-3268`, which is itself the point: the compiler
names every site when a variant is added, so the match does not have to live beside the enum.

**`crates/playback/km-cdg/src/audio.rs:289-296`** — the densest combinator chain in the repo:

```rust
let sample_rate = format.tracks().iter()
    .find(|track| track.id == track_id)
    .and_then(|track| track.codec_params.as_ref())
    .and_then(|params| params.audio())
    .and_then(|params| params.sample_rate)
    .ok_or_else(|| CdgError::no_audio_track(name))?;
```

Four fallible steps, one error, one expression. `and_then` is monadic bind and behaves exactly like
chained `optional::and_then`; `ok_or_else` converts `Option` to `Result`; `?` propagates.

**`crates/song/km-song/src/text.rs:71`** — `?` on an `Option`. This surprises everybody who met `?`
on `Result` first: `let first = folded.chars().next()?;` returns `None` from the enclosing function.
`?` is not "unwrap or throw" — it is "return early with the empty/error case", and it works on any
type that opts in.

**Error plumbing:** `crates/song/km-song/src/lib.rs:50` uses `#[from]`, which generates the
`From` impl that makes `?` auto-convert between error types. Then
`crates/machine/km-api/src/error.rs:172-241` — **four** hand-written `impl From<…> for ApiError`, for when
the mapping is not one-to-one. Reading them in that order shows you what `#[from]` was doing.

**`let … else`**, at `crates/machine/km-queue/src/queue.rs:143` and `crates/machine/karaokemachine/src/machine.rs:1761`:

```rust
let Some(from) = self.entries.iter().position(|e| e.id == id) else {
    return false;
};
```

This is the guard clause you already write, and it is worth learning early because it is
everywhere and it reads badly until you have seen it once.

### The architecture of errors

More valuable than any of the syntax above, and a decision you can carry to your own code. The rule
this workspace holds to throughout:

> **Library crates use `thiserror`. Binary crates use `anyhow`.**

`anyhow` appears in **twelve** manifests — the four binaries `karaokemachine`, `km-remote`,
`km-remote-core` and `km-tray`, and every tool: the four under `tools/cmd/`, `km-pick` under
`tools/dev/`, and three more in the separate `tools/cmd/assets/` workspace. Every `km-*` library
another crate depends on uses `thiserror` only, **29** error enums in all. The reasoning is that a
library's error type is part of its public contract and callers must be able to match on it; a
binary's error is going to be printed to a human, so a single opaque type with context attached is
better.

Then the seam. **`crates/machine/karaokemachine/src/remote.rs:102`** is `fn api_failed(ApiError) -> RemoteError`,
a hand-written translation collapsing HTTP-shaped errors into page-shaped ones — including matching
on the *string code* inside `Conflict { code, message }` at `:110`. Read
`crates/machine/km-api/src/machine.rs:82-86` immediately afterwards, which argues the opposite case: why
`CatalogError` is deliberately coarse, because "nothing useful can be done with a distinction
finer than these".

And a short list of **errors that are deliberately not errors**, which is where the exception
instinct most needs adjusting:

- `crates/machine/km-api/src/events.rs:144-145` — nobody subscribed to the event stream. Normal.
- `crates/remote/km-remote-core/src/client.rs:536` — losing the connection to the machine. *"A machine
  under a television is switched off most of the day."*
- `crates/playback/km-audio/src/audio.rs:442` — an ALSA xrun is not a dead device. This comment documents a
  real bug caused by treating every error alike.

### What your C++ instinct gets wrong here

- **`?` is not an exception.** There is no unwinding, no catch, no cost when it does not fire, and
  no invisible control flow — every propagation point is a visible `?` in the source.
- **`Result` is not an error code you can ignore.** It is `#[must_use]`, so ignoring it is a
  warning, and getting the value out requires you to say what happens in the other case.
- **`Option<&T>` is a null pointer that the compiler makes you check**, and it is the same size as
  a pointer. There is no overhead to pay for the safety.
- **An enum variant can hold different data per variant**, which `enum class` cannot, so a great
  many things you would model as a class hierarchy are an enum here — and get exhaustiveness
  checking for free.
- **Exhaustiveness is the feature.** The compiler telling you about all the places a new variant
  needs handling is worth more than any amount of runtime dispatch.

---

## Stage 4 — Traits and generics

> **The C++ sentence.** You have virtual functions, abstract base classes, and templates. Rust has
> one feature where you have two, and it is neither of them. **Spend the most time here.** This is
> the largest conceptual delta, and the place where a wrong model will cost you the most.

A trait is not an abstract base class. It is not part of the type's layout; it adds nothing to the
object; it can be implemented for a type you did not write, after the fact, in a different crate;
and the choice between static and dynamic dispatch is made at the **use** site rather than baked
into the class by the presence of `virtual`.

### Read

**`crates/playback/km-audio/src/source.rs:14-21`** — `pub trait AudioSource: MidiSink`, three methods.

Then **`crates/playback/km-audio/src/player.rs:31`**, a one-line comment that answers the question you are
about to ask:

> `// MidiSink is not imported: its methods reach us through AudioSource's supertrait bound.`

`: MidiSink` is a **supertrait bound**, and it is the closest thing here to inheritance. It says
"anything implementing `AudioSource` must also implement `MidiSink`" — a requirement, not a base
class, and no layout is shared.

**`crates/playback/km-audio/src/source.rs:264, 313, 383, 427`** — two concrete types (a real SoundFont
synthesiser and a sine-tone test double) each implementing both traits, in four separate `impl`
blocks. The blocks being separate from the type declarations is the demonstration: `SoundFontSource`
does not know it is an `AudioSource` in the way a derived class knows its base.

That test double is worth a moment on its own. It is what lets the whole render pipeline be tested
in CI where no `.sf2` file exists and no audio device is available — and it required no interface
extraction, no dependency injection framework, and no change to `Player`.

**Static against dynamic dispatch, same repository, same problem:**

- `crates/playback/km-audio/src/player.rs:75` — `pub struct Player<S: AudioSource>`, with
  `impl<S: AudioSource> Player<S>` at `:108`. Monomorphised, exactly like a C++ template, one copy
  of the code per concrete `S`, everything inlinable.
- `crates/machine/km-api/src/server.rs:45` — `catalog: Arc<dyn Catalog>`, with `Arc<dyn Controller>`
  beside it at `:46`. Type-erased, one copy of the code, a vtable, and the concrete type not known
  until run time. Four more at `crates/remote/km-remote-pages/src/lib.rs:186-199`, two of them
  wrapped in `Option` because the page works without them.

Read them back to back. Same language feature, two dispatch strategies, chosen per use.

**`crates/machine/km-api/src/machine.rs:919-1168`** — a trait with **default method bodies** (`:963-965`,
and `:1019-1022` which defaults to returning an error). Also the `: Send + Sync + 'static` supertrait
bound at `:919`, which is what makes `Arc<dyn Catalog>` shareable across threads — the bound is on
the trait, so every implementation must satisfy it and no caller has to check.

At 250 lines this is also the repository's clearest case for default bodies: most implementors want
none of the package-problem surface, and a default that refuses is what keeps them from writing an
`unimplemented!()` each.

**Bounds as parameters** — the Rust answer to overloading:

- `impl AsRef<Path>` at `crates/playback/km-audio/src/source.rs:148` — takes a `&str`, a `String`, a
  `Path`, or a `PathBuf`, with one implementation and no overload set. Again at `:197`.
- `impl Into<String>` at `crates/machine/km-api/src/discover.rs:163-164`.
- `<T: Template>` at `crates/remote/km-remote-pages/src/views.rs:45`, and twice more below it.

**Wrapper types:** `crates/playback/km-audio/src/source.rs:137-140` — `pub struct Bank`, holding an
`Arc<rustysynth::SoundFont>` and the defects found in it, whose stated purpose (`:134-135`) is
*dependency hiding*, so that the machine can hold a parsed SoundFont for the life of the process
without naming `rustysynth` in its own manifest.

**The traits you already own, renamed:**

| You know it as | Here | Anchor |
|---|---|---|
| `operator<<` | `Display` | `crates/song/km-songcode/src/lib.rs:163-167` |
| converting constructor | `From` | `crates/machine/km-api/src/dto.rs:53` |
| default constructor | `Default` | derived at `km-queue/src/transport.rs:12`; hand-written eight times in `karaokemachine/src/settings.rs`, the first at `:124` |
| **destructor** | **`Drop`** | `crates/playback/km-cdg/src/audio.rs:335-342` |

`Drop` is the one concept that transfers intact — it is RAII, it runs at end of scope, it runs on
the way out of a panic. The example joins a decoder thread, and the doc at `:274-283` warns it must
never be dropped on the audio callback, which is the same reasoning you would apply to a destructor
that blocks.

### Smart pointers

| Rust | Nearest C++ | Anchor and the point |
|---|---|---|
| `Box<T>` | `unique_ptr<T>` | `crates/playback/km-cdg/src/audio.rs:285` — owned, erased, and *moved to a thread*, which is exactly why it is `Box` and not `Arc`; the argument is spelled out at `:275` |
| `Box<T>` for size | — | `crates/playback/km-audio/src/player.rs:60`, rationale at `:52-54` — boxing one large enum variant so the enum stays small |
| `Arc<T>` | `shared_ptr<const T>` | `crates/playback/km-audio/src/source.rs:137-140` — note it is **immutable by default**, which `shared_ptr` is not |
| `Arc<Mutex<T>>` | *nothing* | `crates/machine/karaokemachine/src/engine.rs:168`, `:171` — shared *mutable* state; the mutex is inside the pointer, so there is no way to reach the data without taking the lock |
| `MutexGuard<'_, T>` | `lock_guard` | `crates/machine/karaokemachine/src/machine.rs:2338`, `:2344` — the `'_` ties the guard to the `&self` it came from, so the type system enforces what `lock_guard` enforces only by convention |
| `Cow<'static, str>` | — | `crates/machine/km-api/src/discover.rs:118-122` — three fields that are a literal in the common case and an owned `String` when the settings name one, with no second type and no allocation for the common case |

**And a thing that is not here at all: there is no `Rc` and no `RefCell` anywhere in this
workspace.** Do not conclude that single-threaded shared ownership does not exist in Rust — it does,
and `Rc`/`RefCell` are how you spell it. It is absent here because everything shared in this design
crosses a thread boundary, so it is `Arc` plus `Mutex`/`RwLock` throughout. You will have to learn
that pair somewhere else; see Appendix D.

### What your C++ instinct gets wrong here

- **A trait is not a base class.** It adds nothing to the layout, there is no `vptr` in the object,
  and `Arc<dyn Trait>` is a fat pointer carrying the vtable beside the data pointer.
- **You can implement a trait for a type you did not write.** This is the thing with no C++
  analogue at all, and it is why `impl Display for SongCode` needs no cooperation from anybody.
- **Generics are checked at the definition, not at instantiation.** `fn f<S: AudioSource>(s: S)` is
  type-checked once, against the bound, before any caller exists. The entire "template error novel"
  problem simply does not occur — and the price is that you must state the bound, where C++ lets
  you rely on whatever the body happens to use.
- **There is no SFINAE, no partial specialisation, no CRTP.** If you find yourself reaching for
  them, the answer is usually a trait with a default method.
- **`dyn` is opt-in, not the default.** In C++ one `virtual` makes every call through the base
  dynamic; here the same trait is static or dynamic depending on how the caller spells the type.

---

## Stage 5 — Iterators and closures

> **The C++ sentence.** You know `<algorithm>`, you know lambdas, and if you have used ranges you
> know most of this already. It is mostly renaming — with two real surprises.

### Read

**`crates/song/km-song/src/text.rs` — all 149 lines.** Small, complete, and it contains
`fn fold_char(ch: char) -> impl Iterator<Item = char>` at `:86`, which is a return type meaning
"some iterator, I am not telling you which". `flat_map` at `:53`, and `String::with_capacity` at
`:51`, which is `reserve` and means the same thing.

**`crates/machine/km-queue/src/queue.rs:100-102`** — two lines that teach a lot:

```rust
pub fn entries(&self) -> impl Iterator<Item = &QueueEntry> {
    self.entries.iter()
}
```

An anonymous return type with an elided lifetime. The iterator borrows `self`, and the compiler
works out that the returned iterator cannot outlive it without anybody writing `'a`.

**`crates/song/km-suitability/src/melody.rs:151-239`** — a successive-narrowing pipeline written as *named
stages with early-return guards* rather than one unreadable chain. This is the house style and it
is a good one: `filter`/`collect` at `:152-155`, `into_iter().map()` at `:165-170`,
`iter().copied().filter()` at `:172-176` and `:187-191`, then `retain` at `:206`.

Two details worth stopping on. `.copied()` at `:174` and `:189` turns an iterator of `&&T` into one
of `&T` — you will meet double references constantly and this is the fix. And `:217`:

```rust
candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
```

Floats do not implement `Ord` in Rust, because NaN makes the ordering not a total order. `sort` is
therefore unavailable and `total_cmp` is the explicit opt-in. C++ lets you sort floats and lets you
have NaN, and does not connect the two.

**The turbofish, in two eight-line functions.** `collect` is generic over what it builds, so
something has to say which container — either the context or you.
`crates/song/km-catalog/src/search.rs:250-258` writes `collect::<Vec<_>>()` and then `.join(", ")` on
it, where nothing downstream would have pinned the type; `crates/playback/km-display/src/draw.rs:1531-1538`
writes `collect::<String>()` and adds an ellipsis to it. Compare either against the plain `collect()`
calls in `melody.rs` above, which need no annotation because the `let` binding already carries one.

**`crates/playback/km-cdg/src/audio.rs:301-318`** — the best `move` closure here, and the second surprise:

```rust
let thread = std::thread::Builder::new().spawn({
    let stop = Arc::clone(&stop);
    let owned = name.to_owned();
    move || { … }
})
```

**There is no capture list.** `move` is all-or-nothing: the closure takes ownership of everything
its body mentions. So when you need to capture *a clone* of one thing and move another, you open a
block, prepare named locals, and let `move` take those. That block-expression-that-prepares-clones
is an idiom you will write hundreds of times, and it is unguessable from C++, where the capture
list does this job.

Compare `crates/machine/karaokemachine/src/engine.rs:282-297`, which does the same thing with locals prepared
before the call rather than inside a block.

### What your C++ instinct gets wrong here

- **There is no capture list.** See above. This is the big one.
- **Iterators are lazy and compose into a single loop.** `.iter().map().filter().collect()` is not
  four passes and not four temporaries; it monomorphises into one loop with no allocation until
  `collect`. They are much closer to C++20 ranges than to `<algorithm>` over a container.
- **A closure is a unique anonymous type**, not a `std::function`. Passing one costs nothing;
  storing one needs `impl Fn` (static) or `Box<dyn Fn>` (dynamic), and that choice is yours.
- **`&&T` is normal, not a mistake.** It comes from `iter()` over a slice of references, and
  `.copied()` or `.cloned()` is the answer.
- **Floats have no `Ord`**, and this will stop you the first time you try to sort by one.

---

## Stage 6 — Modules, crates, features

> **The C++ sentence.** You have headers, translation units, `#ifdef`, and a build system that
> knows nothing about any of them. Rust has modules the compiler understands, crates as the unit of
> both compilation and distribution, and conditional compilation that is checked rather than
> textual.

### Read

**`crates/playback/km-audio/src/lib.rs:28-45`** — eight `pub mod` lines (`:28-35`) followed by seven
`pub use` re-exports (`:37-45`). That second block is the crate's public surface: callers write
`km_audio::Player`, not `km_audio::player::Player`. This is the pattern to copy — organize the source
however you like and present a flat API. Note the counts do not match: `level` is `pub mod` and is
deliberately not re-exported, so a caller who wants it has to name the module and say so.

Read the `//!` block above it while you are there, because it makes a *second* point about that
surface: the queue, the mics and the transport are now `km-queue` and are deliberately **not**
re-exported here. Flattening your own modules is good manners; flattening somebody else's crate
into your namespace hides a dependency, and the whole reason `km-queue` exists is that hiding
this one had `km-display` and both remotes linking a synthesizer they never call.

**`crates/song/km-songbook/src/lib.rs`** — the same shape at 137 lines (`:98-107`), plus something C++ has
no equivalent of at all: **a doc test at `:62-87`**. That is a runnable example inside a comment,
and `cargo test` compiles and runs it. If the API changes and the example stops compiling, the test
suite fails. Documentation that cannot rot.

**`crates/machine/karaokemachine/src/lib.rs:28-71`** — the contrast: **21** private `mod` against **3**
`pub mod`. A module is private by default, so this is the normal case and `km-audio` above is the
deliberate one. It is also the clearest picture of what a binary crate's root looks like: almost
everything in it is an implementation detail, and the three that are not are the ones another crate
or a test reaches for.

**`pub(crate)`** — `crates/playback/km-cdg/src/audio.rs:266` and `:284`. Visible
within this crate, invisible outside it, and there are about 150 across the workspace. It is the
middle ground C++ has no spelling for: `private:` is per-class and a header is per-file, where this
is per-*crate* and the compiler enforces it across every module in one.

**Conditional compilation, the canonical pattern.** `crates/machine/karaokemachine/src/video.rs:23`:

```rust
#[cfg(feature = "video")]
mod …            // the real implementation
#[cfg(not(feature = "video"))]
mod …            // the same public API, returning an error at :159 and :172
```

**Two modules with the same public API, one chosen at compile time.** Everything downstream is
written once against that API and knows nothing about the choice. This is compile-time polymorphism
with no runtime cost and no virtual anything — the job `#ifdef` does in C++, except that the
non-selected branch is still parsed, still name-resolved, and cannot rot silently. Same shape twice
in `tools/cmd/km-pack/src/build.rs`, at `:498`/`:525` and `:615`/`:685`.

Then `crates/machine/km-api/src/lib.rs:52-53` for a conditional *module*:

```rust
#[cfg(any(test, feature = "testing"))]
pub mod testing;
```

— compiled for this crate's own tests *and* for anybody who asks for the feature. And
`crates/platform/km-tray/src/lib.rs` as the densest `#[cfg(target_os = …)]` file, which notably uses
`cfg_attr(…, allow(dead_code))` at `:275`, `:370` and `:389` rather than cfg-ing items away — the
reason is at `:94`, and it is that code which is compiled everywhere cannot rot on the platform you
are not standing on.

**Features, declared:** `crates/playback/km-video/Cargo.toml:14-16` (default-off `ffmpeg`, with a
comment explaining the workspace-member problem it solves),
`crates/machine/karaokemachine/Cargo.toml:64` (`video = ["dep:km-video", "dep:km-stream"]` — one
switch turning on two crates, because a machine that decodes video is also the one that streams it),
`tools/cmd/km-package-builder/Cargo.toml:40-42` (forwarding a feature through to a dependency).

**The root `Cargo.toml`, read top to bottom as a document.** It is the best-commented manifest in
the project and it teaches three things at once: the `[workspace.dependencies]` version table with
a paragraph of justification per entry; `features = ["ffmpeg"]` placed on the *workspace dependency
entry* so no consumer has to name it; and the `exclude = ["tools/cmd/assets"]` note, which is
the clearest real-world explanation of **Cargo feature unification** you will find — a tool that
needed TLS and a tool that needed none were unified into one build, and the one with none got half
a TLS stack and panicked at run time.

**`crates/machine/karaokemachine/build.rs`** — build scripts, which C++ has no direct equivalent of: a Rust program
that runs before your crate compiles and prints instructions to cargo on stdout. Ninety
lines here, and the teaching point is the **two-guard problem**, explained in the comment at
`:46-52`:

- `#[cfg(windows)]` at `:53` and `:57` answers *"is `winresource` even in this build?"* — a **host**
  question, because the dependency is declared under `[target.'cfg(windows)'.build-dependencies]`.
- `CARGO_CFG_TARGET_OS` at `:59` answers *"should I attach an icon resource?"* — a **target**
  question.

Getting one right and not the other is what turned CI red. Cross-compilation makes host and target
two different questions, and a build script is the one place you have to keep them apart by hand.

**Where the tests live.** Beside the code, in `#[cfg(test)] mod tests`, some 3,600 of them across
243 modules in 201 files. Then the crates that split tests into sibling *files* rather than inline
modules (`crates/playback/km-cdg/src/audio/tests.rs`, and `graphics/tests.rs` beside it), and one
integration test worth reading: `crates/machine/km-api/tests/surface.rs`, whose `send` helper at
`:246-258` drives the entire HTTP router through `tower::ServiceExt::oneshot` with no socket at all
— a whole API surface tested without binding a port or opening an audio device. It is 4,649 lines,
so read the `Harness` at the top and one test, not the file.

### What your C++ instinct gets wrong here

- **Module paths are not directories and not files.** `mod` declares a module; where its source
  lives is a separate convention. There are no include guards, no include order, and no ODR.
- **The crate, not the file, is the compilation unit.** Everything in a crate is compiled together,
  which is why cross-module inlining needs no header and why crates are the granularity of parallel
  builds. This is also why splitting a workspace into many small crates is a build-time decision as
  much as a design one.
- **`#[cfg]` is not `#ifdef`.** The disabled branch is still parsed and name-resolved, so it cannot
  quietly stop compiling while you are not looking at it.
- **Features are additive and unify across the whole build.** If two crates in one build ask for
  different feature sets of a shared dependency, they get the union — never two copies. The
  `km-wallpaper-pack` comment in the root manifest is what that costs when you get it wrong.
- **Doc tests run.** Examples in comments are part of the test suite.

---

## Appendix A — The mistranslation table

The fifteen places where the C++ reflex is wrong rather than merely absent. This is the page to
re-read.

Anchors here are shortened to `<crate>/src/<file>` to keep the table narrow. Each resolves to exactly
one file under `crates/`, so `git ls-files '*/km-audio/src/player.rs'` finds it.

| Your C++ reflex says | What Rust actually means | Anchor |
|---|---|---|
| A move calls a move constructor and leaves a valid object | A move is a memcpy, runs no user code, and the source is statically unusable afterwards | `km-songbook/src/arrange.rs:73` |
| `&` is a reference; `const&` is a read-only one | `&` is shared (many allowed); **`&mut` is exclusive (exactly one)** | `km-queue/src/queue.rs:83-165` |
| I should write copy/move ctors and assignment operators | They do not exist as a concept. `Clone` is an ordinary method you call by name | `km-songcode/src/lib.rs:114` |
| `Copy` is like `clone()` but cheaper | `Copy` means implicit memcpy duplication, no user code, and is mutually exclusive with `Drop` | `km-queue/src/transport.rs:9` |
| A trait is an abstract base class | It adds nothing to layout, can be implemented after the fact, for types you did not write | `km-audio/src/source.rs:14` |
| `virtual` makes the whole hierarchy dynamic | `dyn` is chosen per use site; the same trait is static in one place and dynamic in another | `km-audio/src/player.rs:75` vs `km-api/src/server.rs:45` |
| Templates fail at instantiation, in a novel | Generics are checked once at the definition against the stated bound | `km-audio/src/player.rs:108` |
| `'static` means static storage duration | Usually a **bound** meaning "contains no borrows" — what a value needs to cross a thread | `km-kmpkg/src/lib.rs:571-600` |
| `drop(x)` destroys x for its side effects | It ends a **borrow**; it is a scope tool | `karaokemachine/src/machine.rs:1784` |
| A lambda has a capture list | There is no capture list. `move` is all-or-nothing; prepare named clones in a block | `km-cdg/src/audio.rs:301-318` |
| `?` is a hidden throw | Ordinary control flow, visible at every propagation point, no unwinding | `km-songcode/src/lib.rs:194` |
| `optional<T&>` is awkward and nullable | `Option<&T>` is a null pointer the compiler makes you check, at zero size cost | `km-queue/src/queue.rs:105` |
| An index where I expected a pointer is a code smell | Self-referential structs are inexpressible; an index is the idiom | `km-display/src/keypad.rs:169` |
| `#ifdef` deletes code before the compiler sees it | `#[cfg]` still parses and name-resolves the disabled branch | `karaokemachine/src/video.rs:23`/`:141` |
| `unsafe` turns the checks off | It permits five specific operations. Borrowck, lifetimes and type checking all still apply | `km-console/src/lib.rs:177` |

---

## Appendix B — What this path skips, and where it is when you want it

Stopping at Stage 6 is a decision, not a gap. Meeting `Pin` before `&mut` is internalised is how
people bounce off Rust. When you come back, start from these files rather than from a search.

**Threads, `Send`/`Sync`, atomics.** The audio path is the most interesting code in the repository
and it contains **no `Arc<Mutex>` at all**. Three mechanisms, each with a stated reason:
atomics publish state *out* of the callback (`crates/playback/km-audio/src/audio.rs:152-207`, with
`Relaxed` for position at `:615-624` against `AcqRel` for `songs_ended` at `:627`); `rtrb`
single-producer lock-free rings carry commands *in* (`:390-391`); and a third ring exists purely to
move `Drop` — megabyte-sized frees — off the real-time thread.
`crates/playback/km-audio/src/track.rs:47-55` is a hand-rolled generation-counter seek handshake:
the consumer bumps `seek_request` at `:166`, the producer echoes `seek_acked` at `:132`, and the
comparison the consumer reads is at `:171-172`. `crates/machine/karaokemachine/src/engine.rs:1-8`
states the `Send` argument in prose: the cpal stream is created on its own thread and never moves,
because whether it is `Send` depends on the backend, so a design that moved it would compile here
and fail elsewhere. Lock poisoning is handled uniformly and never `.unwrap()`ed — another concept
C++ does not have.

**Async.** `crates/machine/km-api/examples/dev_server.rs` (158 lines) is the entry point — the whole HTTP
API against fixtures. Then `crates/machine/karaokemachine/src/lib.rs:203-722`, which holds the entire
sync/async/GUI seam: a tokio runtime built at `:405` and hosted as a *component* of a synchronous
program whose main thread belongs to SDL. At 520 lines it is a function to navigate rather than
read straight through — find the runtime and read outwards from it. `#[tokio::main]` is deliberately
avoided in every GUI binary and both of them say why.

**`unsafe`.** `crates/platform/km-console/src/lib.rs:159-230` is the workspace's one whole-module allowance
— 309 lines in the file, one `#![allow(unsafe_code)]` at `:177`, two calls, and a long argument for
why it exists at all. Everything else is 39 `#[expect(unsafe_code, reason = "…")]` on single items.
`crates/playback/km-audio/src/level.rs` is the densest with nine, and
`crates/machine/karaokemachine/src/androidctx.rs` the richest in prose, with a SAFETY comment per
operation. Note before you start: `unsafe_code = "deny"` is set workspace-wide in the root manifest,
so every one of these is a deliberate, argued exception.

**FFI**, and it has moved to the ports. Seventeen `extern "C"` across nine files and six
`extern "system"` in one: the Swift-facing exports in `crates/remote/km-remote-ios/src/ffi.rs` and
`crates/machine/km-machine-ios/src/ffi.rs`, and the six JNI exports in
`crates/remote/km-remote-android/src/ffi.rs`, which are `"system"` rather than `"C"` because that is
the ABI the JVM calls on. Note the two `tests/header_matches.rs` beside the iOS ones: a test that
checks a hand-written C header against the Rust exports, which is the seam C++ leaves to the
compiler and Rust leaves to you.

The interesting case is the one with none: `km-video` writes **zero** `unsafe` while binding ffmpeg,
and the reason is at `crates/playback/km-video/src/lib.rs:246`, which is itself the lesson about
where a safe wrapper belongs.

**Macro authoring.** Not one `macro_rules!` in the whole workspace, which is why Stage 2's sidebar
had to quote an outside example. This repository is the wrong text for it.

---

## Appendix C — Runnable entry points

Reading-only does not mean never running anything. When a stage's ideas want confirming, these are
the smallest programs here that produce something you can look at:

| What | Lines | Produces |
|---|---|---|
| `cargo run -p km-songbook --example sample` | 200 | A PDF song book. No I/O framework, no dependencies |
| `cargo run -p km-audio --example render_wav -- in.kar out.wav` | 197 | A WAV. No audio device needed |
| `cargo run -p km-song --example write_fixtures` | 34 | The tiniest runnable thing in the repository |
| `cargo run -p km-lyrics -- dump <file.kar>` | 1,367 | A parsed lyric timeline as JSON |

There are a dozen more `examples/` across the workspace — census programs over the corpus, a
wallpaper preview, an offline CDG decoder. `cargo run -p <crate> --example` with no name lists a
crate's own.

---

## Appendix D — What this codebase cannot teach you

A reader who learns Rust only from this repository will have specific blind spots. Verified absent
from `crates/` and `tools/`, every one of them:

- **`Rc`, `RefCell`, `Weak`** — single-threaded shared ownership and interior mutability. Absent
  because everything shared in this design crosses a thread, so it is `Arc` + `Mutex` throughout.
  This is a real and common part of Rust. *The Rust Book*, ch. 15.
- **`UnsafeCell`, `transmute`, `MaybeUninit`, `unsafe impl Send`/`Sync`** — the actual hard parts
  of `unsafe`. Nothing here does any of it. *The Rustonomicon.*
- **`crossbeam`, `parking_lot`** — the ecosystem's alternative concurrency primitives. Neither is in
  any manifest here: the synchronisation is `std` plus `rtrb`, with `tokio` above it wherever there
  is a server.
- **Procedural macros.** None are authored here. Heavily *used* (serde, thiserror, askama), never
  written. *The Little Book of Rust Macros*, then `syn`/`quote`.
- **Property-based and benchmark testing** — no `proptest`, no `criterion`, no `rstest`, no
  `insta`, no `mockall`. Every mock here is a hand-written trait impl, which is a legitimate style
  but not the only one.

Everything else you need is in the six stages above.
