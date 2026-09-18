//! Turning a parsed [`Pattern`] back into canonical text.

use crate::parser::{CharClass, ClassItem, Component, Pattern, Segment, SegmentKind};

/// Render a parsed pattern back into its canonical textual form.
///
/// The result is a normalized version of the input: runs of bare `*`
/// mixed into a segment are collapsed to one, and anything that needs
/// escaping to round-trip correctly (a literal `*`, `?`, `[`, `{`, `}`,
/// `,`, or `\`) is re-escaped. Feeding the output back into
/// [`crate::parser::parse`] yields the same structure.
pub fn pretty_print(pattern: &Pattern) -> String {
    pattern
        .segments
        .iter()
        .map(print_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn print_segment(segment: &Segment) -> String {
    match &segment.kind {
        SegmentKind::Recursive => "**".to_string(),
        SegmentKind::Parts(components) => {
            components.iter().map(print_component).collect::<String>()
        }
    }
}

fn print_component(component: &Component) -> String {
    match component {
        Component::Literal(s) => escape_literal(s),
        Component::AnyChar => "?".to_string(),
        Component::Star => "*".to_string(),
        Component::Class(class) => print_class(class),
        Component::Alternation(branches) => print_alternation(branches),
    }
}

fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        // '{', '}', and ',' are only meaningful inside a brace group, but
        // escaping them here unconditionally is always safe (an escaped
        // char that has no special meaning where it lands still parses
        // back to itself) and keeps this function context-free.
        if matches!(c, '*' | '?' | '[' | '\\' | '{' | '}' | ',') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn print_alternation(branches: &[Vec<Component>]) -> String {
    let mut out = String::from("{");
    for (i, branch) in branches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&branch.iter().map(print_component).collect::<String>());
    }
    out.push('}');
    out
}

fn print_class(class: &CharClass) -> String {
    let mut out = String::from("[");
    if class.negated {
        out.push('!');
    }
    for item in &class.items {
        match item {
            ClassItem::Char(c) => out.push_str(&escape_class_char(*c)),
            ClassItem::Range(lo, hi) => {
                out.push_str(&escape_class_char(*lo));
                out.push('-');
                out.push_str(&escape_class_char(*hi));
            }
            ClassItem::Posix(class) => {
                out.push_str("[:");
                out.push_str(class.name());
                out.push_str(":]");
            }
        }
    }
    out.push(']');
    out
}

fn escape_class_char(c: char) -> String {
    if matches!(c, ']' | '\\') {
        format!("\\{}", c)
    } else {
        c.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn print(pattern: &str) -> String {
        pretty_print(&parse(pattern).unwrap())
    }

    #[test]
    fn literal_passes_through() {
        assert_eq!(print("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn collapses_runs_of_stars_mixed_into_a_segment() {
        assert_eq!(print("a**b"), "a*b");
        assert_eq!(print("a****b"), "a*b");
    }

    #[test]
    fn keeps_a_whole_segment_of_stars_as_recursive() {
        assert_eq!(print("a/**/b"), "a/**/b");
        assert_eq!(print("a/***/b"), "a/**/b");
    }

    #[test]
    fn any_char_passes_through() {
        assert_eq!(print("a?c"), "a?c");
    }

    #[test]
    fn class_normalizes_and_reorders_items() {
        assert_eq!(print("[cba]"), "[abc]");
        assert_eq!(print("[a-cb-d]"), "[a-d]");
        assert_eq!(print("[[:digit:]a[:alpha:]]"), "[a[:alpha:][:digit:]]");
    }

    #[test]
    fn negated_class_keeps_bang_form() {
        assert_eq!(print("[^abc]"), "[!abc]");
    }

    #[test]
    fn escapes_special_characters_in_literals() {
        assert_eq!(print("a\\*b"), "a\\*b");
        assert_eq!(print("a\\?b"), "a\\?b");
        assert_eq!(print("a\\[b"), "a\\[b");
        assert_eq!(print("a\\{b"), "a\\{b");
        assert_eq!(print("a\\}b"), "a\\}b");
        assert_eq!(print("a\\,b"), "a\\,b");
        assert_eq!(print("a\\\\b"), "a\\\\b");
    }

    #[test]
    fn escapes_bracket_and_backslash_inside_a_class() {
        assert_eq!(print("[a\\]b]"), "[\\]ab]");
        assert_eq!(print("[a\\\\b]"), "[\\\\ab]");
    }

    #[test]
    fn alternation_prints_each_branch() {
        assert_eq!(print("{a,b,c}"), "{a,b,c}");
        assert_eq!(print("*.{rs,toml}"), "*.{rs,toml}");
    }

    #[test]
    fn nested_alternation_keeps_its_own_braces() {
        assert_eq!(print("{a,{b,c}}"), "{a,{b,c}}");
    }

    #[test]
    fn readme_example_round_trips_to_its_normalized_form() {
        assert_eq!(print("a**b/[cba]"), "a*b/[abc]");
    }

    #[test]
    fn output_reparses_to_the_same_structure() {
        for pattern in ["src/**/*.rs", "a\\*b/[a-c\\]]", "{a,b/c}", "[[:alpha:]_]*"] {
            let first = parse(pattern).unwrap_or_else(|e| panic!("{:?} failed to parse: {}", pattern, e));
            let printed = pretty_print(&first);
            let second = parse(&printed).unwrap_or_else(|e| {
                panic!("re-parsing {:?} (from {:?}) failed: {}", printed, pattern, e)
            });
            assert_eq!(first, second);
        }
    }
}
