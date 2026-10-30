//! Repository tasks that cargo can't do on its own.
//!
//! `cargo xtask hardware [--check]` writes README.md's hardware table from
//! `hardware/` and `drivers.toml`, or checks it's current: see [`hardware`].
//!
//! `cargo xtask coverage` checks requirement coverage. It reads every
//! requirement ID in the spec and Owly's implementation notes ([`DOCS`]),
//! every `spec:` citation in the workspace's Rust files, and
//! `tests/coverage.toml`. It fails if an in-scope MUST has no citing test and
//! no policy entry, or if anything names an ID the docs don't define.
//! Out-of-scope MUSTs and untested SHOULDs are reported, not failed.
//!
//! A test cites what it checks with a comment such as `// spec: T-DISC-1` or
//! `// spec: ASK-7, HTTP-3`. A `T-*` ID stands for the requirements its row in
//! `docs/conformance.md`'s test table lists.

mod hardware;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args[..] {
        ["coverage"] => coverage(root),
        ["hardware"] => hardware::run(root, false),
        ["hardware", "--check"] => hardware::run(root, true),
        _ => Err("usage: cargo xtask coverage | hardware [--check]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Must,
    Should,
    Informative,
}

#[derive(Debug, Default)]
struct Spec {
    requirements: BTreeMap<String, Level>,
    /// `T-*` test IDs and the requirement IDs each one covers.
    tests: BTreeMap<String, Vec<String>>,
}

/// The docs that define requirement IDs and the `T-*` tests.
const DOCS: [&str; 3] = [
    "docs/SPEC.md",
    "docs/implementation.md",
    "docs/conformance.md",
];

fn coverage(root: &Path) -> Result<(), String> {
    let mut text = String::new();
    for doc in DOCS {
        text.push_str(&read(&root.join(doc))?);
        text.push('\n');
    }
    let spec = parse_spec(&text);
    let config = parse_config(&read(&root.join("tests/coverage.toml"))?)
        .map_err(|e| format!("tests/coverage.toml: {e}"))?;

    let mut errors = Vec::new();
    let mut cited = BTreeSet::new();
    let mut files = Vec::new();
    for dir in ["crates", "tests"] {
        rust_files(&root.join(dir), &mut files);
    }
    for file in &files {
        let text = read(file)?;
        for (n, line) in text.lines().enumerate() {
            let Some((_, ids)) = line.split_once("// spec:") else {
                continue;
            };
            let at = format!("{}:{}", file.strip_prefix(root).unwrap().display(), n + 1);
            for id in ids.split([',', ' ']).filter(|s| !s.is_empty()) {
                if let Some(covers) = spec.tests.get(id) {
                    cited.extend(covers.iter().cloned());
                } else if spec.requirements.contains_key(id) {
                    cited.insert(id.to_string());
                } else {
                    errors.push(format!("{at}: cites {id}, which the docs don't define"));
                }
            }
        }
    }

    let in_scope: BTreeSet<_> = config.in_scope.iter().cloned().collect();
    for id in in_scope.iter().chain(config.policy.keys()) {
        if !spec.requirements.contains_key(id) {
            errors.push(format!(
                "tests/coverage.toml names {id}, which the docs don't define"
            ));
        }
    }

    let mut report: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, &level) in &spec.requirements {
        let status = if cited.contains(id) {
            "covered by a test"
        } else if config.policy.contains_key(id) {
            "covered by policy"
        } else if level == Level::Must && in_scope.contains(id) {
            errors.push(format!("{id} is in scope but no test cites it"));
            continue;
        } else if level == Level::Must {
            "MUST, out of scope"
        } else if level == Level::Should {
            "SHOULD, untested"
        } else {
            continue;
        };
        report.entry(status).or_default().push(id);
    }
    for (status, ids) in &report {
        println!("{status} ({}): {}", ids.len(), ids.join(" "));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\nerror: "))
    }
}

#[derive(Debug)]
struct Config {
    /// Requirement IDs whose MUSTs need a test. Only ever grows.
    in_scope: Vec<String>,
    /// MUSTs no test can cover, each with the reason.
    policy: BTreeMap<String, String>,
}

fn parse_config(text: &str) -> Result<Config, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    let in_scope = table
        .get("in_scope")
        .and_then(|v| v.as_array())
        .ok_or("in_scope must be an array of IDs")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(String::from)
                .ok_or("in_scope must hold strings")
        })
        .collect::<Result<_, _>>()?;
    let policy = table
        .get("policy")
        .and_then(|v| v.as_table())
        .ok_or("[policy] must be a table of ID = reason")?
        .iter()
        .map(|(id, reason)| match reason.as_str() {
            Some(reason) if !reason.trim().is_empty() => Ok((id.clone(), reason.to_string())),
            _ => Err(format!("policy entry {id} needs a reason")),
        })
        .collect::<Result<_, _>>()?;
    Ok(Config { in_scope, policy })
}

/// A doc defines a requirement as a paragraph starting with its bold ID,
/// such as `**ASK-2a** ...`. Its text runs to the next requirement or
/// heading, and its level is the strongest RFC 2119 keyword in that text.
fn parse_spec(text: &str) -> Spec {
    let mut spec = Spec::default();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        if let Some(id) = line
            .strip_prefix("**")
            .and_then(|rest| rest.split_once("**"))
            .map(|(id, _)| id)
            .filter(|id| is_requirement_id(id))
        {
            finish(&mut current, &mut spec);
            current = Some((id.to_string(), line.to_string()));
        } else if line.starts_with('#') {
            finish(&mut current, &mut spec);
        } else if let Some((_, body)) = &mut current {
            body.push('\n');
            body.push_str(line);
        }
        if let Some(row) = line.strip_prefix("| T-") {
            let mut cells = row.split('|');
            let id = format!("T-{}", cells.next().unwrap_or("").trim());
            let covers = cells.next().unwrap_or("");
            let covers = covers.split([',', ' ']).filter(|s| is_requirement_id(s));
            spec.tests.insert(id, covers.map(String::from).collect());
        }
    }
    finish(&mut current, &mut spec);
    spec
}

fn finish(current: &mut Option<(String, String)>, spec: &mut Spec) {
    let Some((id, body)) = current.take() else {
        return;
    };
    if body.contains("*withdrawn*") {
        return;
    }
    let level = if body.contains("MUST") {
        Level::Must
    } else if body.contains("SHOULD") {
        Level::Should
    } else {
        Level::Informative
    };
    spec.requirements.insert(id, level);
}

/// `ASK-7`, `HTTP-2a`: an uppercase prefix, a dash, a number and an
/// optional lowercase suffix.
fn is_requirement_id(s: &str) -> bool {
    let Some((prefix, number)) = s.split_once('-') else {
        return false;
    };
    let digits = number.trim_end_matches(|c: char| c.is_ascii_lowercase());
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_uppercase())
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}
