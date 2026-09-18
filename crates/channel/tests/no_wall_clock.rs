//! `docs/design/WIRE.md`: no layer in this repo may read physical time.
//! Same guard pattern as muaddib's `clock/tests/no_wall_clock.rs`.
//! Promoted (ticket 003) from scanning only `channel`'s own `src/` to
//! every crate's `src/` in the workspace, now that there's more than one
//! crate to scan — a per-crate copy would just be N places to forget to
//! extend.

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "std::time",
    "SystemTime",
    "Instant",
    "UNIX_EPOCH",
    "chrono",
    "time::OffsetDateTime",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn code_part(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

#[test]
fn no_workspace_crate_source_reads_physical_time() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for krate in fs::read_dir(crates_dir).unwrap() {
        let src = krate.unwrap().path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(
        files.len() >= 2,
        "scanned suspiciously few files ({}), is the path right?",
        files.len()
    );

    let mut violations = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = code_part(line);
            for needle in FORBIDDEN {
                if code.contains(needle) {
                    violations.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "physical-time APIs used in workspace crate source:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_scanner_would_catch_a_violation() {
    assert!(FORBIDDEN
        .iter()
        .any(|n| code_part("let t = std::time::Instant::now();").contains(n)));
    assert!(!FORBIDDEN
        .iter()
        .any(|n| code_part("// never call Instant::now()").contains(n)));
}
