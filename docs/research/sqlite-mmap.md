# How the curation database's pages reach the program

**Findings.** Two questions were open: whether `db::tune`'s memory mapping pays, and what the status
bar's aggregates cost on a whole corpus. Both are answered for reads below, and both answers are the
opposite of what the reasoning predicted. `db::tune` is unchanged, and the one change these figures
argue for has not been made — see the last section.

## What is set now

`db::tune` asks for `mmap_size = 1073741824` — a gibibyte — alongside a page cache sized for what the
connection is for, `temp_store = MEMORY` and `busy_timeout`. Each is best-effort, because an
in-memory database and one on a network share are both real.

Under the mapping, SQLite reads a page by touching mapped memory rather than by asking for it. Two
consequences follow, and neither is visible from inside the program. A page that is not resident
arrives as a fault, one at a time. There is no queue, and no readahead the program can ask for. And
those reads bypass the page cache set beside the mapping, so the two settings are not additive the
way they read.

**That reasoning predicted the mapping would be a liability on a mechanical disk.** It is not.

## How these were measured

`db::measure`, which is two `#[ignore]`d tests over a real corpus. The module's own header says why
it is a child of `db` rather than one of the census examples, and `BUILDING.md` has the commands.

The regime is the one that matters and the one that is hard to get: **the standby list emptied before
every run**, through `NtSetSystemInformation`'s `MemoryPurgeStandbyList`. Warm figures cannot answer
the mapping question at all, because warm is precisely the case where no fault happens. Three
repeats of each setting, alternating, against a copy of a corpus-sized database on a spinning disk.
Round-to-round spread inside a setting is under 8% on every figure below, so the medians are reported
without a range.

**A purge is verified, not assumed.** The first attempt at this used RAMMap without `-accepteula`,
so it emptied nothing and six rounds came back warm while reporting themselves cold. What catches
that is the standby figure read back after the purge, and the cold-to-warm ratio within a run.

## A cold figure has to be checked against the drive

**The first set of "cold" runs here were warm, and the tell was arithmetic.** They reported the five
aggregates at 37.8 ms with the standby list emptied and the purge verified. But a separate
measurement had this drive serving **about twenty reads a second, forty-eight milliseconds apiece**,
at queue depth one. Thirty-eight milliseconds is less than one of those. No sequence of real disk reads fits in
it, so the data was resident however emptied the standby list looked.

So a purge that reports success is necessary and not sufficient. **The second check is whether the
figure is physically possible on the disk underneath**, and it is the one that caught this.

## What a page costs

Warm, and reproducible across rounds:

| | mapped | unmapped |
|---|---|---|
| the status bar's five aggregates | **9.9 ms** | 39.3 ms |
| the count that labels a browse page | **4.6 ms** | 6.2 ms |
| a first page of rows | **67.0 ms** | 222.8 ms |

**Warm, the mapping pays** — 4× on the status bar and 3× on a page of rows, in every round. The
prediction that it would be a liability is wrong here: the copy it avoids is worth more than the
readahead it gives up.

Cold splits in two, and which half a figure lands in depends on how many rows it touches.

**The aggregates are unresolved cold.** `favorites` moved by three orders of magnitude between rounds
of the same setting: twenty seconds, then thirteen, then twelve milliseconds. Its rows are a small
set that stays cached once read, and only a few hundred megabytes come back into the cache between
purges. A cold run's cost is however many of its lookups are still resident, so three rounds
measure this drive's mood rather than the mapping.

**A page of rows is the figure that repeats**, and it is the one worth quoting. It took 30.26 s,
36.26 s and 36.04 s mapped, against 51.98 s, 51.41 s and 36.67 s unmapped. It touches a hundred rows
and their subqueries, far more than stays cached, so it lands in the same place every time. It
favours the mapping by about 1.4×, with one of the six rounds a tie. That is enough to say the
mapping does not hurt cold, and not enough to put a ratio in a decision.

**What cold does say, unambiguously, is which shape of query is expensive:**

| cold, mapped | round 1 | round 2 |
|---|---|---|
| songs | 1.57 s | 5.5 ms |
| files | 5.54 s | 3.0 ms |
| failed | 43.0 ms | 0.226 ms |
| **favorites** | **20.19 s** | **13.54 s** |
| packages | 0.151 ms | 0.227 ms |
| **a first page of rows** | **30.26 s** | **36.26 s** |
| the count that labels it | 6.8 ms | 8.6 ms |

**Everything a covering index answers is milliseconds even cold. Everything that visits rows at
random is seconds.** `count_favorites` is one primary-key lookup per favorited song. Its own note
argues for that shape, which is right warm and pathological here. A few hundred lookups at this
drive's seek time is the twenty seconds measured.

A page of rows is the same shape a hundred times
over, with each row carrying subqueries for its copies and its nicest path. It is the slowest thing on
the page by a wide margin in every round.

That is the finding to carry forward, and it is not about the mapping at all. **Seeks against rows bound a
cold browse of this corpus, and no pragma changes that.** Every index added here — the
browse indexes, `files_failed`, `songs_countable` — has moved a count or a sort. The rows themselves
have never been the thing measured, and on this evidence they are now the whole of the wait.

**The two counts that were expected to dominate are the two the planner serves best:**

```
SELECT COUNT(*) FROM songs WHERE merged_into IS NULL
SEARCH songs USING COVERING INDEX songs_merged (merged_into=?)

SELECT COUNT(*) FROM files
SCAN files USING COVERING INDEX files_status
```

A *covering* index answers both without touching a row, which is why a count of every file in a
corpus costs single-digit milliseconds. **Serving those two a known number of seconds old would buy
nothing**, and the idea is dropped for them. `favorites` is the one in that bar worth attention, and
for the opposite reason: it is cheap warm and seconds cold. What it wants is a shape that does not
seek per row, not a staleness allowance.

**The count that labels a page had no index at all**, and was the worst thing on the page:

```
SELECT COUNT(*) FROM songs s WHERE s.merged_into IS NULL AND s.duplicate_of IS NULL
SCAN s
```

Two terms on two separately indexed columns defeat both single-column indexes. So the planner scanned
the table, every row of the widest one there is. Thirteen seconds cold, forty milliseconds warm,
against one millisecond for the rows beside it, which have a `LIMIT` and a browse index.

**`songs_countable` fixes it, and the shape that works is not the obvious one.** The first attempt was
the partial-index shape `songs_unfolded` takes — `ON songs(id) WHERE <the two
terms>` — and the planner refused it. Forced with `INDEXED BY`, it reported `SCAN s USING INDEX`
without `COVERING`. The predicate guarantees the terms, but the generated code still reads them off
the row. So it was an index scan plus a lookup per row, honestly worse than the table scan. Holding the
two columns themselves is what makes the count answerable without a row:

```
SELECT COUNT(*) FROM songs s WHERE s.merged_into IS NULL AND s.duplicate_of IS NULL
SEARCH s USING COVERING INDEX songs_countable (merged_into=? AND duplicate_of=?)
```

**13.28 s to 6.8 ms cold, 40.7 ms to 4.6 ms warm**, and the planner chooses it unforced. Selectivity
is irrelevant and would condemn the index anywhere else — both columns are NULL on nearly every song,
so the seek returns nearly everything. What makes it cheap is being two narrow columns instead of the
widest table in the schema.

Building it costs one pass and a fresh `ANALYZE` on the first open, 2.47 s on a warm corpus-sized
database. The `ANALYZE` is needed because the planner will not choose an index with no `sqlite_stat1`
row.

## The cache on the status bar

| what | took |
|---|---|
| `counts()` | 25.1 ms |
| again, nothing written since | 0.006 ms |
| again, with the cache cleared | 23.9 ms |

Four thousand times apart, so the cache works and a miss costs what the five aggregates cost. During
a scan every page load is a miss, because the writer's commits move `data_version`. That costs tens
of milliseconds per page, and it is not the problem it was taken for.

## What this argues for, and has not been done

**The two remaining seek-per-row shapes** are now the whole of what a cold page waits on. One is
`count_favorites`, one primary-key lookup per favorited song. The other is a page's rows, each
carrying subqueries for its copies and its nicest path. Both are optimal warm, and seeks bound both
cold. Nothing here changes either: a query rewrite wants its own measurement, and the figures above
are the argument for taking one.

Nothing here argues for touching `mmap_size`. Warm it helps, cold is unresolved, and no proposal
depends on the answer.

## What is still open

**The write half.** A scan *writes* hundreds of rows per batch, scattered by construction because a
song's id is the hash of its bytes. Whether the mapping pays for that is not answered here.

`db::measure`'s second test measures a bounded forced pass, and it has an honest difficulty. A sample
small enough to run in minutes never evicts the database on a machine with this much memory. So it
would compare two settings with the file resident either way, and find nothing for a reason unrelated
to the question. Sizing the sample past the cache means reading tens of gigabytes of song bytes per
run.

The read answer does not carry over. Writes descend the same B-trees to modify them, under a
transaction, with the page cache doing work the mapping bypasses. And the architecture note already
records a ratio pointing the other way for a deep reader queue with the mapping on.

**Verified here:** every figure in the tables, and every query plan quoted, read back from the
database rather than reasoned about. That `songs_countable` is chosen unforced and is covering, and
that the planner refused the partial-index shape once built. That the mapping in force was read back
from the connection, and that the standby list was emptied before each cold round.

**Inferred:** that `count_favorites` and a page's rows are slow cold *because* of their per-row
lookups. The shape of each query says so and the magnitudes fit this drive's seek time, but neither
has been rewritten and measured against itself.

**Withdrawn:** the first cold comparison of the mapping. Its figures were warm, for the reason the
second section gives. The ratio taken from them said the opposite of what the real cold rounds say. A
cold reading on this disk carries a factor of five between rounds, so three rounds do not settle what
the mapping does cold.
