# The sentence shapes of `How a document in this repository is written`, for `check-prose.sh`.
#
# It reads one markdown file, or a commit message on standard input. It prints one hit per line as
# `<line>|<rule>|<text>`. The caller decides which hits to report, and it owns the scope filter.
#
# **`-v page=1` reads a page of HTML instead**, and the caller passes it only for a file
# `prose-converted.txt` names. The other tracked pages are Fluent templates, where a line is markup
# and a sentence counter would report the markup. A tag becomes a space, a closing `</p>`, `</li>` or
# heading closes a paragraph, and a `<script>`, a `<style>`, a comment and a heading's own words are
# never read.
#
# **`-v ftl=1` reads a Fluent catalog**, where the words inside a program live. A message, a term and
# an attribute each open a value, and each value is a paragraph of its own. A placeable is one word,
# as a code span is. A comment there is the catalog's own commentary and stays outside the shape.
#
# **A selector is read at its default variant, and the others are passed over.** A plural's arms
# differ by a word, so reading them all would count one sentence several times and report a length
# no reader meets.
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
  # The mark a block's end leaves behind, once the tags are gone. No page holds this byte.
  SEP = "\001"
}

page { readpage($0); next }
ftl  { readftl($0); next }

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

# **A line of nothing but links is navigation, and an index is made of them.** The link text there is
# a heading somewhere else, and a heading keeps the voice it has, so counting its words would ask an
# index to reword the entries it points at.
navigation($0) { flush(); next }

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
  # **A code span wraps over a line like any other run of words**, and the half on the second line is
  # not prose. Without this the two halves join and the sentence around them is counted wrong.
  if (span) {
    if (match(s, /`/)) { s = substr(s, RSTART + 1); span = 0 } else return ""
  }
  gsub(/`[^`]*`/, "CODE", s)
  if (match(s, /`/)) { s = substr(s, 1, RSTART - 1) " CODE"; span = 1 }
  gsub(/https?:\/\/[^ \t)]*/, "URL", s)
  gsub(/!\[[^]]*\]\([^)]*\)/, "", s)
  gsub(/\]\([^)]*\)/, "]", s)
  gsub(/[][]/, "", s)
  gsub(/\*/, "", s)
  gsub(/[0-9]+(\.[0-9]+)+/, "NUM", s)
  gsub(/([eE]\.g|[iI]\.e|etc|vs)\./, "ABBR", s)
  return s
}

function navigation(s) {
  gsub(/!?\[[^]]*\]\([^)]*\)/, "", s)
  gsub(/[ \t·,;|*_—-]/, "", s)
  return s == ""
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

# **A page is read in one pass per line, and the state it carries is what a tag can span.** A
# comment, a `<script>`, a `<style>` and a heading each run over as many lines as they like, and so
# does a single tag with its attributes.
function readpage(line,   n, parts, i) {
  if (comment) {
    if (match(line, /-->/)) { line = substr(line, RSTART + 3); comment = 0 } else return
  }
  line = uncomment(line)
  if (skipping) {
    if (match(line, /<\/[ \t]*(script|style)[ \t]*>/)) {
      line = substr(line, RSTART + RLENGTH)
      skipping = 0
    } else return
  }
  line = drop(line, "<[ \t]*(script|style)[^>]*>", "</[ \t]*(script|style)[ \t]*>", SEP)
  if (dangling) skipping = 1
  if (tag) {
    if (match(line, />/)) { line = substr(line, RSTART + 1); tag = 0 } else return
  }
  # A heading instructs, so its words stay outside the shape, exactly as a Markdown heading does. It
  # holds tags of its own here -- the page's own `<h1>` carries an `<img>` and a `<span>` -- so the
  # pair is found by searching past the opening rather than by matching the whole element at once.
  if (heading) {
    if (match(line, /<\/[ \t]*h[1-6][ \t]*>/)) {
      line = SEP substr(line, RSTART + RLENGTH)
      heading = 0
    } else return
  }
  line = drop(line, "<[ \t]*h[1-6][^>]*>", "</[ \t]*h[1-6][ \t]*>", SEP)
  if (dangling) heading = 1
  # A `<code>` span is one word, as a Markdown code span is. A path or a flag is an exact item, and
  # counting its parts would ask a page to spell one shorter.
  if (code) {
    if (match(line, /<\/[ \t]*code[ \t]*>/)) { line = substr(line, RSTART + RLENGTH); code = 0 } else return
  }
  line = drop(line, "<[ \t]*code[^>]*>", "</[ \t]*code[ \t]*>", " CODE ")
  if (dangling) code = 1
  gsub(/<[ \t]*br[^>]*>/, SEP, line)
  gsub(/<\/[ \t]*(p|li|td|th|div|blockquote|figcaption|section|main|header|footer|dd|dt)[ \t]*>/, SEP, line)
  gsub(/<[^>]*>/, " ", line)
  if (match(line, /<[^>]*$/)) {
    line = substr(line, 1, RSTART - 1)
    tag = 1
  }
  gsub(/&[a-zA-Z]+;/, " ", line)
  gsub(/&#[0-9]+;/, " ", line)
  n = split(line, parts, SEP)
  for (i = 1; i <= n; i++) {
    add(FNR, parts[i])
    if (i < n) flush()
  }
}

# **Every element of one kind goes, and `dangling` says whether one is left open.** An element whose
# pair sits on one line takes its words with it; one that opens and does not close hands the caller a
# state to carry into the next line.
function drop(line, opener, closer, mark,   head, rest) {
  dangling = 0
  while (match(line, opener)) {
    head = substr(line, 1, RSTART - 1)
    rest = substr(line, RSTART + RLENGTH)
    if (match(rest, closer)) {
      line = head mark substr(rest, RSTART + RLENGTH)
    } else {
      dangling = 1
      return head mark
    }
  }
  return line
}

# **A catalog is read one value at a time, and the state it carries is which value is open.** A
# message, a term and an attribute each close the value before them, so each is a paragraph and
# carries its own six sentences. An indented line continues the value above it.
function readftl(line,   text) {
  if (line ~ /^[ \t]*#/)  { flush(); return }
  if (line ~ /^[ \t]*$/)  { flush(); return }
  if (line ~ /^-?[a-zA-Z][a-zA-Z0-9_-]*[ \t]*=/ ||
      line ~ /^[ \t]*\.[a-zA-Z][a-zA-Z0-9_-]*[ \t]*=/) {
    flush()
    sub(/^[^=]*=[ \t]*/, "", line)
  } else if (line ~ /^[ \t]*\*\[/) {
    sub(/^[ \t]*\*\[[^]]*\][ \t]*/, "", line)
  } else if (line ~ /^[ \t]*\[/) {
    return
  }
  text = placeables(line)
  # A selector closes on a line of its own, and what it leaves behind is punctuation rather than a
  # word. Counting that would add one to the sentence it closes.
  if (text ~ /^[ \t]*[)\].,;:]*[ \t]*$/) return
  add(FNR, text)
}

# **A placeable is one word, as a code span is.** A variable, a term and a function call are exact
# items the decision says keep their spelling. It takes no padding, so the comma after one stays on
# the word it follows rather than becoming a word of its own.
#
# **A selector's opening leaves nothing behind**, because the variant below it carries the same
# placeable again. Emitting one here would count the plural's own number twice.
function placeables(s,   before) {
  do {
    before = s
    gsub(/\{[^{}]*\}/, "CODE", s)
  } while (s != before)
  gsub(/\{[^{}]*->[ \t]*$/, "", s)
  gsub(/\{[^{}]*$/, "CODE", s)
  gsub(/[{}]/, " ", s)
  return s
}

# A comment closed on the line it opens on, as many times as it appears; one left open sets the
# state the next line reads.
function uncomment(line,   head, rest) {
  while (match(line, /<!--/)) {
    head = substr(line, 1, RSTART - 1)
    rest = substr(line, RSTART)
    if (match(rest, /-->/)) {
      line = head " " substr(rest, RSTART + 3)
    } else {
      comment = 1
      return head
    }
  }
  return line
}
