use std::env;
use std::fs;
use std::process::ExitCode;

use globlint::parser::{self, GlobError};
use globlint::printer::pretty_print;

struct Report {
    pattern: String,
    valid: bool,
    normalized: Option<String>,
    error: Option<GlobError>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut json = false;
    let mut fix = false;
    let mut patterns = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "--json" => json = true,
            "--fix" => fix = true,
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            other => patterns.push(other.to_string()),
        }
    }

    if patterns.is_empty() {
        eprintln!("error: no pattern given\n");
        print_usage();
        return ExitCode::from(2);
    }

    if fix {
        return run_fix(&patterns);
    }

    let reports: Vec<Report> = patterns
        .into_iter()
        .map(|pattern| match parser::parse(&pattern) {
            Ok(parsed) => Report {
                pattern,
                valid: true,
                normalized: Some(pretty_print(&parsed)),
                error: None,
            },
            Err(err) => Report {
                pattern,
                valid: false,
                normalized: None,
                error: Some(err),
            },
        })
        .collect();

    let all_valid = reports.iter().all(|r| r.valid);

    if json {
        print_json(&reports);
    } else {
        print_human(&reports);
    }

    if all_valid {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_usage() {
    eprintln!("usage: globlint [--json] <pattern>...");
    eprintln!("       globlint --fix <file>...");
    eprintln!();
    eprintln!("Validate and pretty-print one or more glob patterns.");
    eprintln!();
    eprintln!("  --json     emit a JSON array of results instead of text");
    eprintln!("  --fix      rewrite each <file>'s patterns (one per line) to");
    eprintln!("             their normalized form in place; invalid lines are");
    eprintln!("             left untouched and reported to stderr");
    eprintln!("  -h, --help show this message");
}

/// Rewrite each file's patterns (one per line) to their normalized form.
/// Blank lines are left alone. A line that fails to parse is left as-is
/// and reported to stderr; every other line in the file is still fixed.
fn run_fix(paths: &[String]) -> ExitCode {
    let mut had_error = false;

    for path in paths {
        let contents = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("error: {}: {}", path, err);
                had_error = true;
                continue;
            }
        };

        let mut changed = false;
        let mut fixed_count = 0;
        let mut out_lines = Vec::with_capacity(contents.lines().count());
        for (i, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                out_lines.push(line.to_string());
                continue;
            }
            match parser::parse(line) {
                Ok(parsed) => {
                    let normalized = pretty_print(&parsed);
                    if normalized != line {
                        changed = true;
                        fixed_count += 1;
                    }
                    out_lines.push(normalized);
                }
                Err(err) => {
                    eprintln!("error: {}:{}: {}", path, i + 1, err);
                    had_error = true;
                    out_lines.push(line.to_string());
                }
            }
        }

        if changed {
            let mut new_contents = out_lines.join("\n");
            if contents.ends_with('\n') {
                new_contents.push('\n');
            }
            if let Err(err) = fs::write(path, new_contents) {
                eprintln!("error: {}: {}", path, err);
                had_error = true;
                continue;
            }
            println!("{}: fixed {} pattern(s)", path, fixed_count);
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_human(reports: &[Report]) {
    for (i, report) in reports.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("pattern:    {}", report.pattern);
        println!("valid:      {}", if report.valid { "yes" } else { "no" });
        match &report.error {
            Some(err) => println!("error:      {}", err),
            None => println!(
                "normalized: {}",
                report.normalized.as_deref().unwrap_or("")
            ),
        }
    }
}

fn print_json(reports: &[Report]) {
    let mut out = String::from("[\n");
    for (i, report) in reports.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str("  {\n");
        out.push_str(&format!(
            "    \"pattern\": {},\n",
            json_string(&report.pattern)
        ));
        out.push_str(&format!("    \"valid\": {},\n", report.valid));
        match &report.error {
            Some(err) => {
                out.push_str("    \"normalized\": null,\n");
                out.push_str("    \"error\": {\n");
                out.push_str(&format!("      \"code\": {},\n", json_string(err.code())));
                out.push_str(&format!(
                    "      \"message\": {},\n",
                    json_string(&err.to_string())
                ));
                out.push_str(&format!("      \"position\": {}\n", err.position()));
                out.push_str("    }\n");
            }
            None => {
                out.push_str(&format!(
                    "    \"normalized\": {},\n",
                    json_string(report.normalized.as_deref().unwrap_or(""))
                ));
                out.push_str("    \"error\": null\n");
            }
        }
        out.push_str("  }");
    }
    out.push_str("\n]");
    println!("{}", out);
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
