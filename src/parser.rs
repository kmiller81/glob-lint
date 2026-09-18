//! Parsing and validation of glob patterns.
//!
//! A pattern is split on unescaped `/` into segments. Each segment is
//! either the literal string `**` (or any run of two or more bare `*`,
//! which means the same thing: match zero or more whole path segments)
//! or a sequence of components: literal text, `?`, `*`, `[...]`
//! character classes, and `{...}` alternation groups.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    Literal(String),
    AnyChar,
    Star,
    Class(CharClass),
    /// A `{a,b,c}` group. Each branch is itself a sequence of components,
    /// so branches may contain literals, `?`, `*`, `[...]` classes, and
    /// nested `{...}` groups.
    Alternation(Vec<Vec<Component>>),
}

/// `items` is always in canonical form after parsing: overlapping or
/// adjacent `Char`/`Range` entries are merged and sorted by codepoint, and
/// come before any `Posix` entries, which are themselves sorted and
/// deduplicated by name. See `normalize_items` for the exact rules.
#[derive(Debug, Clone, PartialEq)]
pub struct CharClass {
    pub negated: bool,
    pub items: Vec<ClassItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassItem {
    Char(char),
    Range(char, char),
    Posix(PosixClass),
}

/// One of the standard POSIX named character classes, written `[:name:]`
/// inside a `[...]` bracket expression (e.g. `[[:alpha:][:digit:]_]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosixClass {
    Alnum,
    Alpha,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Xdigit,
}

impl PosixClass {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "alnum" => PosixClass::Alnum,
            "alpha" => PosixClass::Alpha,
            "blank" => PosixClass::Blank,
            "cntrl" => PosixClass::Cntrl,
            "digit" => PosixClass::Digit,
            "graph" => PosixClass::Graph,
            "lower" => PosixClass::Lower,
            "print" => PosixClass::Print,
            "punct" => PosixClass::Punct,
            "space" => PosixClass::Space,
            "upper" => PosixClass::Upper,
            "xdigit" => PosixClass::Xdigit,
            _ => return None,
        })
    }

    /// The name as written between the colons, e.g. `"alpha"`.
    pub fn name(&self) -> &'static str {
        match self {
            PosixClass::Alnum => "alnum",
            PosixClass::Alpha => "alpha",
            PosixClass::Blank => "blank",
            PosixClass::Cntrl => "cntrl",
            PosixClass::Digit => "digit",
            PosixClass::Graph => "graph",
            PosixClass::Lower => "lower",
            PosixClass::Print => "print",
            PosixClass::Punct => "punct",
            PosixClass::Space => "space",
            PosixClass::Upper => "upper",
            PosixClass::Xdigit => "xdigit",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SegmentKind {
    Recursive,
    Parts(Vec<Component>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub kind: SegmentKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GlobError {
    DanglingEscape { pos: usize },
    UnterminatedClass { pos: usize },
    EmptyClass { pos: usize },
    InvalidRange { pos: usize, lo: char, hi: char },
    UnterminatedBrace { pos: usize },
    UnknownPosixClass { pos: usize, name: String },
}

impl GlobError {
    /// A short machine-readable identifier for the error, stable across
    /// versions, meant for the `--json` output mode.
    pub fn code(&self) -> &'static str {
        match self {
            GlobError::DanglingEscape { .. } => "dangling_escape",
            GlobError::UnterminatedClass { .. } => "unterminated_class",
            GlobError::EmptyClass { .. } => "empty_class",
            GlobError::InvalidRange { .. } => "invalid_range",
            GlobError::UnterminatedBrace { .. } => "unterminated_brace",
            GlobError::UnknownPosixClass { .. } => "unknown_posix_class",
        }
    }

    /// Character offset into the original pattern where the problem starts.
    pub fn position(&self) -> usize {
        match self {
            GlobError::DanglingEscape { pos }
            | GlobError::UnterminatedClass { pos }
            | GlobError::EmptyClass { pos }
            | GlobError::InvalidRange { pos, .. }
            | GlobError::UnterminatedBrace { pos }
            | GlobError::UnknownPosixClass { pos, .. } => *pos,
        }
    }
}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GlobError::DanglingEscape { pos } => {
                write!(f, "dangling escape at position {}", pos)
            }
            GlobError::UnterminatedClass { pos } => {
                write!(f, "unterminated character class starting at position {}", pos)
            }
            GlobError::EmptyClass { pos } => {
                write!(f, "empty character class at position {}", pos)
            }
            GlobError::InvalidRange { pos, lo, hi } => write!(
                f,
                "invalid range '{}-{}' at position {} (start is greater than end)",
                lo, hi, pos
            ),
            GlobError::UnterminatedBrace { pos } => {
                write!(f, "unterminated brace group starting at position {}", pos)
            }
            GlobError::UnknownPosixClass { pos, name } => write!(
                f,
                "unknown POSIX class '[:{}:]' at position {}",
                name, pos
            ),
        }
    }
}

/// Parse and validate a glob pattern, producing its component structure.
pub fn parse(pattern: &str) -> Result<Pattern, GlobError> {
    let chars: Vec<char> = pattern.chars().collect();
    let raw_segments = split_segments(&chars);

    let mut segments = Vec::with_capacity(raw_segments.len());
    for (start, seg_chars) in raw_segments {
        let components = tokenize_segment(&seg_chars, start)?;
        let kind = if components.len() >= 2
            && components.iter().all(|c| matches!(c, Component::Star))
        {
            SegmentKind::Recursive
        } else {
            SegmentKind::Parts(collapse_stars(components))
        };
        segments.push(Segment { kind });
    }

    Ok(Pattern { segments })
}

/// Split a pattern into segments on unescaped `/`, ignoring any `/` that
/// falls inside a `[...]` class or a `{...}` brace group (a branch like
/// `{a,b/c}` stays literal text rather than being split further; see the
/// README). Returns each segment's raw characters together with its
/// starting offset in the original pattern, which is used to report
/// absolute error positions later.
fn split_segments(chars: &[char]) -> Vec<(usize, Vec<char>)> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut current_start = 0usize;
    let mut in_class = false;
    let mut brace_depth = 0usize;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            current.push(c);
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '[' && in_class && chars.get(i + 1) == Some(&':') {
            // A `[:name:]` named class nested inside an already-open
            // bracket expression, e.g. `[[:alpha:]]`. Consume it whole so
            // its own `]` doesn't get mistaken for the outer class's
            // closing bracket below.
            current.push(c);
            i += 1;
            while i < chars.len() {
                let cc = chars[i];
                current.push(cc);
                i += 1;
                if cc == ':' && chars.get(i) == Some(&']') {
                    current.push(']');
                    i += 1;
                    break;
                }
            }
            continue;
        }
        if c == '[' && !in_class {
            in_class = true;
            current.push(c);
            i += 1;
            continue;
        }
        if c == ']' && in_class {
            in_class = false;
            current.push(c);
            i += 1;
            continue;
        }
        if c == '{' && !in_class {
            brace_depth += 1;
            current.push(c);
            i += 1;
            continue;
        }
        if c == '}' && !in_class && brace_depth > 0 {
            brace_depth -= 1;
            current.push(c);
            i += 1;
            continue;
        }
        if c == '/' && !in_class && brace_depth == 0 {
            segments.push((current_start, std::mem::take(&mut current)));
            current_start = i + 1;
            i += 1;
            continue;
        }
        current.push(c);
        i += 1;
    }
    segments.push((current_start, current));
    segments
}

fn tokenize_segment(chars: &[char], base: usize) -> Result<Vec<Component>, GlobError> {
    let mut out = Vec::new();
    let mut literal = String::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                if i + 1 >= chars.len() {
                    return Err(GlobError::DanglingEscape { pos: base + i });
                }
                literal.push(chars[i + 1]);
                i += 2;
            }
            '?' => {
                flush_literal(&mut literal, &mut out);
                out.push(Component::AnyChar);
                i += 1;
            }
            '*' => {
                flush_literal(&mut literal, &mut out);
                out.push(Component::Star);
                i += 1;
            }
            '[' => {
                flush_literal(&mut literal, &mut out);
                let (class, consumed) = parse_class(&chars[i..], base + i)?;
                out.push(Component::Class(class));
                i += consumed;
            }
            '{' => {
                flush_literal(&mut literal, &mut out);
                let (branches, consumed) = parse_brace(&chars[i..], base + i)?;
                out.push(Component::Alternation(branches));
                i += consumed;
            }
            _ => {
                literal.push(c);
                i += 1;
            }
        }
    }
    flush_literal(&mut literal, &mut out);
    Ok(out)
}

fn flush_literal(literal: &mut String, out: &mut Vec<Component>) {
    if !literal.is_empty() {
        out.push(Component::Literal(std::mem::take(literal)));
    }
}

/// Parse a `[...]` class starting at `chars[0]` (which must be `[`).
/// Returns the class and the number of characters consumed from `chars`.
///
/// A literal `]` inside a class must be escaped as `\]`; an unescaped `]`
/// always closes the class, there is no special leading-`]` exception.
fn parse_class(chars: &[char], base: usize) -> Result<(CharClass, usize), GlobError> {
    let mut i = 1; // skip '['
    let mut negated = false;
    if let Some(&c) = chars.get(i) {
        if c == '!' || c == '^' {
            negated = true;
            i += 1;
        }
    }

    let mut items = Vec::new();

    loop {
        match chars.get(i) {
            Some(&']') => {
                i += 1;
                break;
            }
            Some(&'[') if chars.get(i + 1) == Some(&':') => {
                let (class, next) = parse_posix_class(chars, i, base)?;
                items.push(ClassItem::Posix(class));
                i = next;
            }
            Some(_) => {
                let (lo, next) = read_class_char(chars, i, base)?;
                i = next;
                if chars.get(i) == Some(&'-') && chars.get(i + 1).map_or(false, |&c| c != ']') {
                    let (hi, next) = read_class_char(chars, i + 1, base)?;
                    if lo > hi {
                        return Err(GlobError::InvalidRange { pos: base, lo, hi });
                    }
                    items.push(ClassItem::Range(lo, hi));
                    i = next;
                } else {
                    items.push(ClassItem::Char(lo));
                }
            }
            None => return Err(GlobError::UnterminatedClass { pos: base }),
        }
    }

    if items.is_empty() {
        return Err(GlobError::EmptyClass { pos: base });
    }

    let class = CharClass {
        negated,
        items: normalize_items(items),
    };
    Ok((class, i))
}

/// Put a class's items into canonical form: overlapping or adjacent
/// `Char`/`Range` items are merged into the smallest set of ranges that
/// covers the same characters, sorted by codepoint, followed by the
/// class's POSIX names (if any), sorted and deduplicated. This never
/// changes which characters the class matches, only how it is written.
fn normalize_items(items: Vec<ClassItem>) -> Vec<ClassItem> {
    let mut posix = Vec::new();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for item in items {
        match item {
            ClassItem::Char(c) => ranges.push((c as u32, c as u32)),
            ClassItem::Range(lo, hi) => ranges.push((lo as u32, hi as u32)),
            ClassItem::Posix(p) => posix.push(p),
        }
    }

    ranges.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (lo, hi) in ranges {
        match merged.last_mut() {
            Some(last) if lo <= last.1.saturating_add(1) => {
                if hi > last.1 {
                    last.1 = hi;
                }
            }
            _ => merged.push((lo, hi)),
        }
    }

    posix.sort_by_key(|p: &PosixClass| p.name());
    posix.dedup();

    let mut out = Vec::with_capacity(merged.len() + posix.len());
    for (lo, hi) in merged {
        // Both endpoints came from valid `char`s, so the merged range's
        // endpoints (a subset spanning between them) are always valid too.
        if lo == hi {
            out.push(ClassItem::Char(char::from_u32(lo).unwrap()));
        } else {
            out.push(ClassItem::Range(
                char::from_u32(lo).unwrap(),
                char::from_u32(hi).unwrap(),
            ));
        }
    }
    for p in posix {
        out.push(ClassItem::Posix(p));
    }
    out
}

/// Read a single, possibly backslash-escaped character inside a class,
/// returning the character and the index just past it.
fn read_class_char(chars: &[char], i: usize, base: usize) -> Result<(char, usize), GlobError> {
    match chars.get(i) {
        Some(&'\\') => match chars.get(i + 1) {
            Some(&c) => Ok((c, i + 2)),
            None => Err(GlobError::DanglingEscape { pos: base + i }),
        },
        Some(&c) => Ok((c, i + 1)),
        None => Err(GlobError::UnterminatedClass { pos: base }),
    }
}

/// Parse a `[:name:]` named class starting at `chars[i]` (which must be
/// `[` followed by `:`), nested inside an already-open `[...]` bracket
/// expression. `base` is the offset of the *outer* class's `[`, used for
/// error positions in the same way the rest of `parse_class` does.
/// Returns the class and the index just past the closing `:]`.
fn parse_posix_class(
    chars: &[char],
    i: usize,
    base: usize,
) -> Result<(PosixClass, usize), GlobError> {
    let mut j = i + 2; // skip "[:"
    let mut name = String::new();
    loop {
        match chars.get(j) {
            None => return Err(GlobError::UnterminatedClass { pos: base }),
            Some(&':') if chars.get(j + 1) == Some(&']') => break,
            Some(&c) => {
                name.push(c);
                j += 1;
            }
        }
    }
    let class = PosixClass::from_name(&name)
        .ok_or(GlobError::UnknownPosixClass { pos: base + i, name })?;
    Ok((class, j + 2))
}

/// Parse a `{...}` alternation group starting at `chars[0]` (which must be
/// `{`). Branches are split on unescaped, top-level commas: a comma inside
/// a nested `[...]` class or a nested `{...}` group does not split. Each
/// branch is tokenized the same way a whole segment is, so a branch may
/// itself contain literals, `?`, `*`, classes, and further nested groups.
/// Returns the branches and the number of characters consumed from `chars`.
fn parse_brace(chars: &[char], base: usize) -> Result<(Vec<Vec<Component>>, usize), GlobError> {
    let mut i = 1; // skip '{'
    let mut depth = 1usize;
    let mut in_class = false;
    let mut branch_start = 1usize;
    let mut current = Vec::new();
    let mut raw_branches: Vec<(usize, Vec<char>)> = Vec::new();

    loop {
        match chars.get(i) {
            None => return Err(GlobError::UnterminatedBrace { pos: base }),
            Some(&'\\') => {
                if i + 1 >= chars.len() {
                    return Err(GlobError::DanglingEscape { pos: base + i });
                }
                current.push('\\');
                current.push(chars[i + 1]);
                i += 2;
            }
            Some(&'[') if in_class && chars.get(i + 1) == Some(&':') => {
                // Same nested `[:name:]` case as in `split_segments`: don't
                // let its `]` close the outer class early.
                current.push('[');
                i += 1;
                while i < chars.len() {
                    let cc = chars[i];
                    current.push(cc);
                    i += 1;
                    if cc == ':' && chars.get(i) == Some(&']') {
                        current.push(']');
                        i += 1;
                        break;
                    }
                }
            }
            Some(&'[') if !in_class => {
                in_class = true;
                current.push('[');
                i += 1;
            }
            Some(&']') if in_class => {
                in_class = false;
                current.push(']');
                i += 1;
            }
            Some(&'{') if !in_class => {
                depth += 1;
                current.push('{');
                i += 1;
            }
            Some(&'}') if !in_class => {
                depth -= 1;
                if depth == 0 {
                    raw_branches.push((branch_start, std::mem::take(&mut current)));
                    i += 1;
                    break;
                }
                current.push('}');
                i += 1;
            }
            Some(&',') if !in_class && depth == 1 => {
                raw_branches.push((branch_start, std::mem::take(&mut current)));
                branch_start = i + 1;
                i += 1;
            }
            Some(&c) => {
                current.push(c);
                i += 1;
            }
        }
    }

    let mut branches = Vec::with_capacity(raw_branches.len());
    for (start, raw) in raw_branches {
        branches.push(collapse_stars(tokenize_segment(&raw, base + start)?));
    }
    Ok((branches, i))
}

/// Collapse runs of two or more bare `*` down to one. A run that fills an
/// entire segment is handled separately as `SegmentKind::Recursive`; this
/// only applies to stars mixed in with other components, e.g. `a**b`.
fn collapse_stars(components: Vec<Component>) -> Vec<Component> {
    let mut out: Vec<Component> = Vec::with_capacity(components.len());
    for c in components {
        if matches!(c, Component::Star) && matches!(out.last(), Some(Component::Star)) {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_items(pattern: &str) -> Vec<ClassItem> {
        match &parse(pattern).unwrap().segments[0].kind {
            SegmentKind::Parts(components) => match &components[0] {
                Component::Class(class) => class.items.clone(),
                other => panic!("expected a class, got {:?}", other),
            },
            other => panic!("expected a Parts segment, got {:?}", other),
        }
    }

    #[test]
    fn merges_overlapping_ranges() {
        assert_eq!(
            class_items("[a-cb-d]"),
            vec![ClassItem::Range('a', 'd')]
        );
    }

    #[test]
    fn merges_adjacent_ranges() {
        assert_eq!(
            class_items("[a-cd-f]"),
            vec![ClassItem::Range('a', 'f')]
        );
    }

    #[test]
    fn does_not_merge_ranges_with_a_gap() {
        assert_eq!(
            class_items("[a-ce-g]"),
            vec![ClassItem::Range('a', 'c'), ClassItem::Range('e', 'g')]
        );
    }

    #[test]
    fn drops_duplicate_chars() {
        assert_eq!(class_items("[aa]"), vec![ClassItem::Char('a')]);
    }

    #[test]
    fn sorts_items_by_codepoint() {
        assert_eq!(
            class_items("[cab]"),
            vec![
                ClassItem::Char('a'),
                ClassItem::Char('b'),
                ClassItem::Char('c')
            ]
        );
    }

    #[test]
    fn posix_classes_sort_after_ranges_and_dedup() {
        assert_eq!(
            class_items("[[:digit:]a[:digit:][:alpha:]]"),
            vec![
                ClassItem::Char('a'),
                ClassItem::Posix(PosixClass::Alpha),
                ClassItem::Posix(PosixClass::Digit)
            ]
        );
    }

    #[test]
    fn single_char_range_collapses_to_char() {
        assert_eq!(class_items("[a-a]"), vec![ClassItem::Char('a')]);
    }

    #[test]
    fn splits_on_unescaped_slashes() {
        let pattern = parse("a/b/c").unwrap();
        assert_eq!(pattern.segments.len(), 3);
    }

    #[test]
    fn escaped_slash_does_not_split_a_segment() {
        let pattern = parse("a\\/b").unwrap();
        assert_eq!(pattern.segments.len(), 1);
    }

    #[test]
    fn slash_inside_a_class_does_not_split_a_segment() {
        let pattern = parse("[a/b]c").unwrap();
        assert_eq!(pattern.segments.len(), 1);
    }

    #[test]
    fn slash_inside_a_brace_group_does_not_split_a_segment() {
        let pattern = parse("{a,b/c}").unwrap();
        assert_eq!(pattern.segments.len(), 1);
    }

    #[test]
    fn a_run_of_two_or_more_bare_stars_is_a_recursive_segment() {
        assert_eq!(parse("**").unwrap().segments[0].kind, SegmentKind::Recursive);
        assert_eq!(parse("***").unwrap().segments[0].kind, SegmentKind::Recursive);
    }

    #[test]
    fn a_single_star_segment_is_not_recursive() {
        assert_eq!(
            parse("*").unwrap().segments[0].kind,
            SegmentKind::Parts(vec![Component::Star])
        );
    }

    #[test]
    fn stars_mixed_with_other_text_collapse_but_stay_a_parts_segment() {
        assert_eq!(
            parse("a**b").unwrap().segments[0].kind,
            SegmentKind::Parts(vec![
                Component::Literal("a".to_string()),
                Component::Star,
                Component::Literal("b".to_string()),
            ])
        );
    }

    #[test]
    fn escape_turns_off_special_meaning() {
        assert_eq!(
            parse("a\\*b").unwrap().segments[0].kind,
            SegmentKind::Parts(vec![Component::Literal("a*b".to_string())])
        );
    }

    #[test]
    fn brace_group_branches_may_nest() {
        let pattern = parse("{a,{b,c}}").unwrap();
        match &pattern.segments[0].kind {
            SegmentKind::Parts(components) => match &components[0] {
                Component::Alternation(branches) => {
                    assert_eq!(branches.len(), 2);
                    assert_eq!(branches[0], vec![Component::Literal("a".to_string())]);
                    assert!(matches!(branches[1][0], Component::Alternation(_)));
                }
                other => panic!("expected an alternation, got {:?}", other),
            },
            other => panic!("expected a Parts segment, got {:?}", other),
        }
    }

    #[test]
    fn dangling_escape_at_end_of_pattern_is_an_error() {
        assert_eq!(parse("a\\").unwrap_err(), GlobError::DanglingEscape { pos: 1 });
    }

    #[test]
    fn unterminated_class_is_an_error() {
        assert_eq!(
            parse("a[bc").unwrap_err(),
            GlobError::UnterminatedClass { pos: 1 }
        );
    }

    #[test]
    fn empty_class_is_an_error() {
        assert_eq!(parse("[]").unwrap_err(), GlobError::EmptyClass { pos: 0 });
    }

    #[test]
    fn descending_range_is_an_error() {
        assert_eq!(
            parse("[z-a]").unwrap_err(),
            GlobError::InvalidRange {
                pos: 0,
                lo: 'z',
                hi: 'a'
            }
        );
    }

    #[test]
    fn unterminated_brace_is_an_error() {
        assert_eq!(
            parse("a{b,c").unwrap_err(),
            GlobError::UnterminatedBrace { pos: 1 }
        );
    }

    #[test]
    fn unknown_posix_class_name_is_an_error() {
        match parse("[[:bogus:]]").unwrap_err() {
            GlobError::UnknownPosixClass { pos, name } => {
                assert_eq!(pos, 1);
                assert_eq!(name, "bogus");
            }
            other => panic!("expected UnknownPosixClass, got {:?}", other),
        }
    }

    #[test]
    fn error_code_and_position_are_exposed() {
        let err = parse("[]").unwrap_err();
        assert_eq!(err.code(), "empty_class");
        assert_eq!(err.position(), 0);
    }

    #[test]
    fn error_display_mentions_the_position() {
        let err = parse("a\\").unwrap_err();
        assert_eq!(err.to_string(), "dangling escape at position 1");
    }
}
