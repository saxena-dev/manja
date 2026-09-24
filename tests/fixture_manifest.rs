//! Provenance checks for the vendored binary ticker corpus
//! (`tests/fixtures/ticker/`, `docs/verification.md` §1.2).
//!
//! The manifest must list every other file in the corpus tree, and nothing
//! else, with a matching SHA-256 and size, so vendored bytes can neither drift
//! nor silently grow. The capture container must frame into exactly 19
//! records with no trailing bytes.

// Shared with other test crates, which use helpers this one does not.
#[allow(dead_code)]
#[path = "support/capture.rs"]
mod capture;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// One `[[file]]` entry of `MANIFEST.toml`.
#[derive(Debug, Default, Clone)]
struct Entry {
    fields: BTreeMap<String, String>,
}

/// Parse the flat TOML subset the manifest uses: comments, `key = "string"`,
/// `key = integer`, and `[[file]]` tables. Anything else is an error.
fn parse_manifest(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut in_file = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[file]]" {
            entries.push(Entry::default());
            in_file = true;
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .ok_or_else(|| format!("line {}: not `key = value`: {raw}", n + 1))?;
        let value = if let Some(s) = value.strip_prefix('"') {
            let s = s
                .strip_suffix('"')
                .ok_or_else(|| format!("line {}: unterminated string", n + 1))?;
            if s.contains('"') {
                return Err(format!("line {}: embedded quote", n + 1));
            }
            s.to_string()
        } else if value.chars().all(|c| c.is_ascii_digit()) {
            value.to_string()
        } else {
            return Err(format!("line {}: unsupported value {value}", n + 1));
        };
        if in_file {
            entries
                .last_mut()
                .unwrap()
                .fields
                .insert(key.to_string(), value);
        } else if key != "manifest_version" || value != "1" {
            return Err(format!("line {}: unexpected top-level key {key}", n + 1));
        }
    }
    Ok(entries)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn files_under(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.retain(|p| p != "MANIFEST.toml");
    out.sort();
    out
}

/// Check the corpus at `root` against its manifest. Returns every problem found.
fn verify(root: &Path) -> Result<usize, Vec<String>> {
    let text = std::fs::read_to_string(root.join("MANIFEST.toml"))
        .map_err(|e| vec![format!("cannot read MANIFEST.toml: {e}")])?;
    let entries = parse_manifest(&text).map_err(|e| vec![e])?;
    let mut problems = Vec::new();
    if entries.is_empty() {
        problems.push("manifest lists no files".to_string());
    }
    let mut listed: Vec<String> = Vec::new();
    for entry in &entries {
        for key in [
            "path",
            "source_repository",
            "source_commit",
            "original_path",
            "sha256",
            "size",
            "provenance",
        ] {
            if !entry.fields.contains_key(key) {
                problems.push(format!("entry {:?} lacks `{key}`", entry.fields));
            }
        }
        if !matches!(
            entry.fields.get("provenance").map(String::as_str),
            Some("captured" | "derived" | "synthetic")
        ) {
            problems.push(format!("entry {:?} has no valid provenance", entry.fields));
        }
        let Some(path) = entry.fields.get("path") else {
            continue;
        };
        if path.ends_with(".json") && !entry.fields.contains_key("generated_by") {
            problems.push(format!("expectation file {path} lacks `generated_by`"));
        }
        listed.push(path.clone());
        match std::fs::read(root.join(path)) {
            Ok(bytes) => {
                let digest = sha256_hex(&bytes);
                if Some(&digest) != entry.fields.get("sha256") {
                    problems.push(format!("{path}: sha256 {digest} does not match manifest"));
                }
                if Some(&bytes.len().to_string()) != entry.fields.get("size") {
                    problems.push(format!("{path}: size {} does not match", bytes.len()));
                }
            }
            Err(e) => problems.push(format!("{path}: {e}")),
        }
    }
    listed.sort();
    let on_disk = files_under(root);
    for path in &on_disk {
        if !listed.contains(path) {
            problems.push(format!("{path} is on disk but not in the manifest"));
        }
    }
    if problems.is_empty() {
        Ok(entries.len())
    } else {
        Err(problems)
    }
}

fn corpus_dir() -> PathBuf {
    capture::ticker_fixtures_dir()
}

/// A scratch copy of the corpus, unique to this process and `tag`.
fn scratch_copy(tag: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("manja-fixtures-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    let src = corpus_dir();
    let mut files = files_under(&src);
    files.push("MANIFEST.toml".to_string());
    for rel in files {
        let to = dst.join(&rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(src.join(&rel), to).unwrap();
    }
    dst
}

#[test]
fn every_vendored_file_matches_its_manifest_entry() {
    let checked = verify(&corpus_dir()).unwrap_or_else(|p| panic!("{}", p.join("\n")));
    // 11 cases × (.bin + .json) plus the real capture.
    assert_eq!(checked, 23);
}

#[test]
fn one_byte_of_drift_fails_the_check() {
    let dir = scratch_copy("drift");
    let path = dir.join("protocol/single_full.bin");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[10] ^= 0x01;
    std::fs::write(&path, bytes).unwrap();
    let problems = verify(&dir).unwrap_err();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("single_full.bin: sha256")),
        "{problems:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unlisted_file_fails_the_check() {
    let dir = scratch_copy("unlisted");
    std::fs::write(dir.join("protocol/extra.bin"), [0u8]).unwrap();
    let problems = verify(&dir).unwrap_err();
    assert!(
        problems
            .iter()
            .any(|p| p.contains("protocol/extra.bin is on disk but not in the manifest")),
        "{problems:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_truncated_manifest_fails_actionably() {
    let dir = scratch_copy("truncated-manifest");
    let manifest = dir.join("MANIFEST.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, &text[..text.len() / 2]).unwrap();
    let problems = verify(&dir).unwrap_err();
    assert!(!problems.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_capture_container_frames_all_19_records() {
    let bytes = capture::read_real_capture();
    let records = capture::read_capture(&bytes).unwrap();
    assert_eq!(records.len(), 19);
    // Every byte is consumed by a header or a payload: 19 twelve-byte headers.
    let payload_bytes: usize = records.iter().map(|r| r.payload.len()).sum();
    assert_eq!(payload_bytes + 19 * 12, bytes.len());
    // Receipt times are non-decreasing in this capture.
    assert!(records
        .windows(2)
        .all(|w| w[0].receipt_unix_nanos <= w[1].receipt_unix_nanos));
    assert_eq!(records[0].receipt_unix_nanos, 1_769_019_551_354_728_000);
    assert_eq!(records[0].payload.len(), 453_458);
}

#[test]
fn the_container_reader_rejects_truncation() {
    let bytes = capture::read_real_capture();
    let cut = &bytes[..bytes.len() - 1];
    assert!(matches!(
        capture::read_capture(cut),
        Err(capture::CaptureError::TruncatedPayload { .. })
    ));
    assert!(matches!(
        capture::read_capture(&bytes[..5]),
        Err(capture::CaptureError::TruncatedHeader { offset: 0 })
    ));
}
