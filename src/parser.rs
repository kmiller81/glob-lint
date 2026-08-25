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

#[derive(Debug, Clone, PartialEq)]
pub struct CharClass {
    pub negated: bool,
    pub items: Vec<ClassItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassItem {
    Char(char),
    Range(char, char),
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
        }
    }

    /// Character offset into the original pattern where the problem starts.
    pub fn position(&self) -> usize {
        match self {
            GlobError::DanglingEscape { pos }
            | GlobError::UnterminatedClass { pos }
            | GlobError::EmptyClass { pos }
            | GlobError::InvalidRange { pos, .. }
            | GlobError::UnterminatedBrace { pos } => *pos,
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

    Ok((CharClass { negated, items }, i))
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
