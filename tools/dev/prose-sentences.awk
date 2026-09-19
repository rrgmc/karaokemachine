# The sentence shapes of `How a document in this repository is written`, for `check-prose.sh`.
#
# It reads one markdown file, or a commit message on standard input. It prints one hit per line as
# `<line>|<rule>|<text>`. The caller decides which hits to report, and it owns the scope filter.
#
# **It is one process per file, where a phrase shape is one `grep` per shape per file.** The
# whole-tree form already spends two thirds of its time creating processes. A second scanner may add
# one process to a file, and it may not add fourteen.
#
# Three rules have a shape here: a sentence over twenty-five words, a paragraph over six sentences,
# and a passive verb that names its agent. Idiom, metaphor and a long noun string have no shape, so a
# human reads for those.
#
# **What it does not read is as deliberate as what it does.** A heading instructs, and the decision
# exempts it. A fenced block is code. A table row is a cell. A quoted block is somebody else's
# sentence. A list item opens a paragraph of its own, because it carries its own six sentences.

BEGIN {
  LIMIT = 25
  PARAGRAPH = 6
  # **A passive verb is caught only where it names its agent**, which is the one form with no
  # ambiguity to argue about. A regular participle ends in `ed` or `en`; the rest are spelled out,
  # because `is read by` is the shape this repository reaches for most.
  IRREGULAR = "read|built|made|held|kept|sent|left|put|set|told|found|lost|meant|run|brought|dealt|felt|split|cut|hit|shut|let|sung|drawn|grown"
  PASSIVE = "(^| )(is|are|was|were|been|being) +([a-z]+(ed|en)|" IRREGULAR ") +by( |$)"
}

/^[ \t]*(```|~~~)/ { flush(); fence = !fence; next }
fence            { next }

# **A block of HTML is markup, and it runs to the blank line after it.** The README draws its
# pictures in a `<table>`, and a cell there is a caption. An `alt` is a label a screen reader speaks,
# and it describes one picture in one breath.
/^[ \t]*</ { flush(); html = 1 }
html && /^[ \t]*$/ { html = 0 }
html       { next }

# An `alt` in Markdown wraps over as many lines as it needs, and it closes at the URL's bracket.
/^[ \t]*!\[/ { flush(); image = 1 }
image        { if (index($0, ")")) image = 0; next }

/^[ \t]*(#|\||>)/                        { flush(); next }
/^[ \t]*(-{3,}|={3,}|\*{3,})[ \t]*$/     { flush(); next }
/^[ \t]*$/                               { flush(); next }

/^[ \t]*([-*+]|[0-9]+\.)[ \t]/ { flush() }

{ add(FNR, $0) }

END { flush() }

# **A code span is one word, and a link is its own text.** Both carry the exact items the decision
# says stay verbatim, and neither is a sentence a reader parses. A version number and an
# abbreviation are masked because a full stop inside one does not end a sentence.
function normalize(s) {
  gsub(/^[ \t]*([-*+]|[0-9]+\.)[ \t]+/, "", s)
  gsub(/`[^`]*`/, "CODE", s)
  gsub(/https?:\/\/[^ \t)]*/, "URL", s)
  gsub(/!\[[^]]*\]\([^)]*\)/, "", s)
  gsub(/\]\([^)]*\)/, "]", s)
  gsub(/[][]/, "", s)
  gsub(/\*/, "", s)
  gsub(/[0-9]+(\.[0-9]+)+/, "NUM", s)
  gsub(/([eE]\.g|[iI]\.e|etc|vs)\./, "ABBR", s)
  return s
}

function words(s,   parts, i, n, c) {
  n = split(s, parts, /[ \t]+/)
  c = 0
  for (i = 1; i <= n; i++)
    if (parts[i] != "") c++
  return c
}

# Each line records the word it starts at, so a sentence reports the line it begins on rather than
# the paragraph's first. `--changed` reads only the lines a branch added, and a paragraph one line
# of which is new is the ordinary case.
function add(line, s,   n) {
  s = normalize(s)
  n = words(s)
  if (n == 0) return
  starts++
  startword[starts] = seen + 1
  startline[starts] = line
  seen += n
  paragraph = paragraph " " s
}

function lineof(word,   i, answer) {
  answer = startline[1]
  for (i = 1; i <= starts; i++)
    if (startword[i] <= word) answer = startline[i]
  return answer
}

function report(line, rule, text) {
  gsub(/[ \t]+/, " ", text)
  sub(/^ /, "", text)
  if (length(text) > 90) text = substr(text, 1, 87) "..."
  print line "|" rule "|" text
}

function flush(   n, i, parts, w, counted, before, line) {
  if (paragraph == "") { reset(); return }
  n = split(paragraph, parts, /[.?!]["')]?([ \t]+|$)/)
  counted = 0
  before = 0
  for (i = 1; i <= n; i++) {
    w = words(parts[i])
    if (w == 0) continue
    counted++
    line = lineof(before + 1)
    if (w > LIMIT)
      report(line, "sentence length", w " words: " parts[i])
    if (parts[i] ~ PASSIVE)
      report(line, "passive voice", parts[i])
    before += w
  }
  if (counted > PARAGRAPH)
    report(startline[1], "paragraph length", counted " sentences")
  reset()
}

function reset() {
  paragraph = ""
  starts = 0
  seen = 0
}
