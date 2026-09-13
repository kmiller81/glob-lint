# globlint

A small tool for checking whether a glob pattern actually means what you
think it means, before you hand it to a build script, a `.gitignore`, or a
config file that expects one.

Glob syntax looks simple until you hit the edges: does `**` need to be its
own path segment? What happens with an unterminated `[abc`? Is `[z-a]` a
typo or a class nobody will ever match? Different tools (shells, `fnmatch`,
gitignore, various language libraries) disagree on these details, and a
pattern that's silently wrong just doesn't match anything, with no error at
all. `globlint` parses a pattern against one fixed, documented set of rules,
tells you exactly what's wrong and where, and prints back the canonical form
so you can see how it was actually understood.

It's a parser, a pretty printer, and a matcher — it does not walk a
filesystem for you (there's no `--match` in the CLI yet, and no directory
traversal at all), but the library can tell you whether a given path would
match a given pattern, and the CLI can normalize a whole file of patterns
with `--fix`.

## Usage

```
$ globlint 'src/**/*.rs'
pattern:    src/**/*.rs
valid:      yes
normalized: src/**/*.rs

$ globlint 'a**b/[z-a]'
pattern:    a**b/[z-a]
valid:      no
error:      invalid range 'z-a' at position 5 (start is greater than end)

$ globlint --json 'src/**/*.rs' 'notes/[unterminated'
[
  {
    "pattern": "src/**/*.rs",
    "valid": true,
    "normalized": "src/**/*.rs",
    "error": null
  },
  {
    "pattern": "notes/[unterminated",
    "valid": false,
    "normalized": null,
    "error": {
      "code": "unterminated_class",
      "message": "unterminated character class starting at position 6",
      "position": 6
    }
  }
]
```

Multiple patterns can be passed on one command line; the process exits `0`
if every pattern given was valid, `1` if any were not, and `2` on a usage
error (no patterns at all).

`--fix <file>...` treats each argument as a path to a file holding one glob
pattern per line, and rewrites every valid pattern in place to its
normalized form. Blank lines are left alone, and a line that fails to parse
is left untouched and reported to stderr rather than blocking the rest of
the file:

```
$ cat patterns.txt
src/**/*.rs
a**b/[cba]
notes/[unterminated

$ globlint --fix patterns.txt
error: patterns.txt:3: unterminated character class starting at position 6
patterns.txt: fixed 1 pattern(s)

$ cat patterns.txt
src/**/*.rs
a*b/[abc]
notes/[unterminated
```

## Syntax

- Any plain character matches itself.
- `?` matches exactly one character.
- `*` matches any run of characters, but never crosses a `/`.
- `**` as an entire path segment (e.g. the `**` in `a/**/b`) matches zero
  or more whole path segments. A run of three or more bare stars filling a
  segment (`***`) is treated the same way. Stars mixed with other text in
  a segment (`a**b`) are just collapsed to a single ordinary `*`.
- `[abc]` matches one character from the set. `[a-z]` matches one from the
  range. `[!abc]` or `[^abc]` negate the set. A literal `]` inside a class
  must be written `\]`. A class can also contain one or more POSIX named
  classes, `[:alpha:]`, `[:digit:]`, `[:alnum:]`, `[:upper:]`, `[:lower:]`,
  `[:space:]`, `[:blank:]`, `[:punct:]`, `[:cntrl:]`, `[:print:]`,
  `[:graph:]`, and `[:xdigit:]`, written inside the surrounding brackets
  (`[[:alpha:]_]` matches a letter or underscore). Note the double
  brackets: `[:alpha:]` on its own, without an enclosing `[...]`, is just
  the literal characters `:`, `a`, `l`, `p`, `h`. A class's contents are
  normalized on parse: overlapping or adjacent characters and ranges are
  merged (`[a-cb-d]` becomes `[a-d]`) and sorted by codepoint, with any
  `[:name:]` classes moved to the end, sorted and deduplicated.
- `{a,b,c}` matches any one of the comma-separated branches. Branches can
  contain anything a segment can (literal text, `?`, `*`, `[...]` classes,
  even a nested `{...}` group); `{a,{b,c}}` is `{a,b,c}` in disguise. A
  literal `{`, `}`, or `,` must be escaped if it isn't meant to take part
  in a group.
- `\` escapes the next character, turning off any special meaning it would
  otherwise have.

Patterns are split into segments on unescaped `/`; a `/` inside `[...]` or
`{...}` is kept as a literal character rather than treated as a separator,
so a branch like `{a,b/c}` stays within one segment instead of being split
into two.

## Errors caught today

- a trailing, unescaped `\` with nothing after it
- a `[` that is never closed
- an empty class, `[]`
- a class range where the start is after the end, like `[z-a]`
- a `{` that is never closed
- a `[:name:]` with a name that isn't one of the twelve standard POSIX classes

Each error reports a character position (0-indexed, counted in Unicode
scalar values) pointing at the start of the problem.

## Library use

The CLI is a thin wrapper around the `globlint` library crate:

```rust
use globlint::{parse, pretty_print};

let pattern = parse("src/**/*.rs")?;
println!("{}", pretty_print(&pattern));
```

`parse` returns a `Result<Pattern, GlobError>`; `Pattern` exposes its
segments and components directly if you want to inspect the structure
instead of just re-printing it.

`is_match` checks a path against an already-parsed pattern:

```rust
use globlint::{parse, is_match};

let pattern = parse("src/**/*.rs")?;
assert!(is_match(&pattern, "src/parser.rs"));
assert!(is_match(&pattern, "src/sub/dir/lib.rs"));
assert!(!is_match(&pattern, "src/main.py"));
```

Matching works on the parsed structure directly: `*` never crosses a `/`,
`**` as a whole segment matches zero or more path segments, and classes,
`?`, and `{...}` alternation all behave the way [Syntax](#syntax) describes.
There is no special-casing of a leading `.` in either the pattern or the
path — if you want to exclude dotfiles you need a pattern that says so.

## Roadmap

Rough order:

- a unit test suite covering the parser and printer (the matcher already has one)
