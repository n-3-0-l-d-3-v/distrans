//! `docs/design/WIRE.md`: no layer in this repo may read physical time.
//! This is the same guard pattern as muaddib's
//! `clock/tests/no_wall_clock.rs`. It currently scans only `channel`'s
//! own `src/`; later crates in this repo should add an equivalent test
//! (or this one should be promoted to a workspace-wide scan once there's
//! more than one crate to scan) rather than assuming the constraint holds
//! by convention.

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
fn no_channel_source_reads_physical_time() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(!files.is_empty());

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
        "physical-time APIs used in channel source:\n{}",
        violations.join("\n")
    );
}
