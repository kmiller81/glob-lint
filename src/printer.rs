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
