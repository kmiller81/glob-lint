//! Testing a parsed [`Pattern`] against an actual path.

use crate::parser::{CharClass, ClassItem, Component, Pattern, PosixClass, Segment, SegmentKind};

/// Does `path` match `pattern`?
///
/// `path` is split into segments on `/`, the same way a pattern is, and
/// matched segment by segment: a `Recursive` (`**`) segment consumes zero
/// or more whole path segments, and a `Parts` segment matches exactly one
/// path segment against its components. There is no special-casing of a
/// leading `.` in either the pattern or the path.
pub fn is_match(pattern: &Pattern, path: &str) -> bool {
    let path_segments: Vec<&str> = path.split('/').collect();
    match_segments(&pattern.segments, &path_segments)
}

fn match_segments(segments: &[Segment], path: &[&str]) -> bool {
    let (seg, rest) = match segments.split_first() {
        Some(pair) => pair,
        None => return path.is_empty(),
    };

    match &seg.kind {
        SegmentKind::Recursive => {
            if match_segments(rest, path) {
                return true;
            }
            match path.split_first() {
                Some((_, path_rest)) => match_segments(segments, path_rest),
                None => false,
            }
        }
        SegmentKind::Parts(components) => match path.split_first() {
            Some((first, path_rest)) => {
                let chars: Vec<char> = first.chars().collect();
                match_components(components, &chars) && match_segments(rest, path_rest)
            }
            None => false,
        },
    }
}

fn match_components(components: &[Component], chars: &[char]) -> bool {
    match_seq(components, chars, &|remaining| remaining.is_empty())
}

/// Continuation-passing matcher: matches `components` against a prefix of
/// `chars`, then calls `k` on whatever is left over. This is what lets
/// `Alternation` branches (which each carry their own sub-sequence of
/// components) be tried without having to splice them into a flat list
/// first: the branch is matched with a continuation that resumes matching
/// `rest` of the outer sequence.
fn match_seq(components: &[Component], chars: &[char], k: &dyn Fn(&[char]) -> bool) -> bool {
    let (head, rest) = match components.split_first() {
        Some(pair) => pair,
        None => return k(chars),
    };

    match head {
        Component::Literal(lit) => {
            let lit_chars: Vec<char> = lit.chars().collect();
            if chars.len() < lit_chars.len() || chars[..lit_chars.len()] != lit_chars[..] {
                return false;
            }
            match_seq(rest, &chars[lit_chars.len()..], k)
        }
        Component::AnyChar => match chars.split_first() {
            Some((_, chars_rest)) => match_seq(rest, chars_rest, k),
            None => false,
        },
        Component::Star => {
            for split in 0..=chars.len() {
                if match_seq(rest, &chars[split..], k) {
                    return true;
                }
            }
            false
        }
        Component::Class(class) => match chars.split_first() {
            Some((&c, chars_rest)) => class_contains(class, c) && match_seq(rest, chars_rest, k),
            None => false,
        },
        Component::Alternation(branches) => branches
            .iter()
            .any(|branch| match_seq(branch, chars, &|remaining| match_seq(rest, remaining, k))),
    }
}

fn class_contains(class: &CharClass, c: char) -> bool {
    let found = class.items.iter().any(|item| match item {
        ClassItem::Char(x) => *x == c,
        ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
        ClassItem::Posix(p) => posix_contains(*p, c),
    });
    found != class.negated
}

fn posix_contains(class: PosixClass, c: char) -> bool {
    match class {
        PosixClass::Alnum => c.is_ascii_alphanumeric(),
        PosixClass::Alpha => c.is_ascii_alphabetic(),
        PosixClass::Blank => c == ' ' || c == '\t',
        PosixClass::Cntrl => c.is_ascii_control(),
        PosixClass::Digit => c.is_ascii_digit(),
        PosixClass::Graph => c.is_ascii_graphic(),
        PosixClass::Lower => c.is_ascii_lowercase(),
        PosixClass::Print => c.is_ascii_graphic() || c == ' ',
        PosixClass::Punct => c.is_ascii_punctuation(),
        PosixClass::Space => c.is_ascii_whitespace(),
        PosixClass::Upper => c.is_ascii_uppercase(),
        PosixClass::Xdigit => c.is_ascii_hexdigit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn matches(pattern: &str, path: &str) -> bool {
        is_match(&parse(pattern).unwrap(), path)
    }

    #[test]
    fn literal_segments() {
        assert!(matches("src/main.rs", "src/main.rs"));
        assert!(!matches("src/main.rs", "src/lib.rs"));
        assert!(!matches("src/main.rs", "src/main.rs/extra"));
    }

    #[test]
    fn star_does_not_cross_slash() {
        assert!(matches("src/*.rs", "src/main.rs"));
        assert!(!matches("src/*.rs", "src/sub/main.rs"));
    }

    #[test]
    fn any_char() {
        assert!(matches("a?c", "abc"));
        assert!(!matches("a?c", "ac"));
        assert!(!matches("a?c", "abbc"));
    }

    #[test]
    fn recursive_segment() {
        assert!(matches("a/**/b", "a/b"));
        assert!(matches("a/**/b", "a/x/b"));
        assert!(matches("a/**/b", "a/x/y/b"));
        assert!(!matches("a/**/b", "a/b/c"));
    }

    #[test]
    fn character_class() {
        assert!(matches("[a-c].rs", "b.rs"));
        assert!(!matches("[a-c].rs", "d.rs"));
        assert!(matches("[!a-c].rs", "d.rs"));
        assert!(matches("[[:digit:]].txt", "5.txt"));
        assert!(!matches("[[:digit:]].txt", "x.txt"));
    }

    #[test]
    fn alternation() {
        assert!(matches("*.{rs,toml}", "lib.rs"));
        assert!(matches("*.{rs,toml}", "Cargo.toml"));
        assert!(!matches("*.{rs,toml}", "README.md"));
        assert!(matches("{a,b}{c,d}", "bc"));
    }

    #[test]
    fn escaped_literal_is_not_special() {
        assert!(matches("a\\*b", "a*b"));
        assert!(!matches("a\\*b", "axb"));
    }
}
