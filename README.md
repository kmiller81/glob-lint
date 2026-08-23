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

It's a parser and a pretty printer — it does not walk a filesystem or match
patterns against paths. That's a natural next step, not this one (see
Roadmap).

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
  must be written `\]`.
- `\` escapes the next character, turning off any special meaning it would
  otherwise have.

Patterns are split into segments on unescaped `/`; a `/` inside `[...]` is
kept as a literal character of the class rather than treated as a
separator.

## Errors caught today

- a trailing, unescaped `\` with nothing after it
- a `[` that is never closed
- an empty class, `[]`
- a class range where the start is after the end, like `[z-a]`

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

## Roadmap

Rough order:

- brace alternation, `{a,b,c}`
- POSIX class names inside brackets, `[:alpha:]` and friends
- an actual matcher: test a pattern against a real path, not just parse it
- normalize character classes (merge overlapping ranges, sort items)
- a `--fix` mode that rewrites a file's patterns to their normalized form in place
