//! Every reference in the repository resolves inside the repository
//! (`docs/verification.md` §7), without a network:
//!
//! - `kite:<page>.md:<lines>` citations name a page listed in
//!   `docs/kite-sources.toml` and stay within its recorded line count, and
//!   every listed page has its URL, access time, SHA-256, size and lines;
//! - `QUOTA_PROFILE_VERSION` names the access date of `exceptions.md`, with
//!   an optional `+r<revision>`;
//! - every bound, coverage and breaking-change ID used is defined in `docs/`;
//! - every `docs/<file>.md §N` reference names an existing section;
//! - no file refers to material that is not part of the repository.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The files whose references are checked, relative to the root.
fn scanned() -> Vec<String> {
    let root = root();
    let mut out = Vec::new();
    for top in [
        "src", "tests", "benches", "examples", "docs", "scripts", ".github",
    ] {
        walk(&root, &root.join(top), &mut out);
    }
    for file in ["README.md", "Cargo.toml"] {
        out.push(file.to_string());
    }
    out.push("tests/fixtures/ticker/MANIFEST.toml".to_string());
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("tests/fixtures") {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, out);
        } else if fs::read_to_string(&path).is_ok() {
            out.push(rel);
        }
    }
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// One `[[page]]` entry of `docs/kite-sources.toml`.
#[derive(Debug, Default)]
struct Page {
    fields: BTreeMap<String, String>,
}

impl Page {
    fn get(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }
}

/// Parse the flat TOML subset the sources manifest uses.
fn kite_pages() -> BTreeMap<String, Page> {
    let text = read("docs/kite-sources.toml");
    let mut pages: Vec<Page> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[page]]" {
            pages.push(Page::default());
            continue;
        }
        let (key, value) = line
            .split_once(" = ")
            .unwrap_or_else(|| panic!("kite-sources.toml:{}: not `key = value`", n + 1));
        let value = value.trim_matches('"').to_string();
        if let Some(page) = pages.last_mut() {
            page.fields.insert(key.to_string(), value);
        } else {
            assert_eq!((key, value.as_str()), ("manifest_version", "1"));
        }
    }
    pages
        .into_iter()
        .map(|p| (p.get("name").to_string(), p))
        .collect()
}

#[test]
fn every_listed_kite_page_is_fully_recorded() {
    let pages = kite_pages();
    assert!(!pages.is_empty());
    for (name, page) in &pages {
        let stem = name.strip_suffix(".md").expect(name);
        assert_eq!(
            page.get("url"),
            format!("https://kite.trade/docs/connect/v3/{stem}/index.md"),
            "{name}"
        );
        let accessed = page.get("accessed");
        assert!(
            accessed.len() == 20 && accessed.as_bytes()[10] == b'T' && accessed.ends_with('Z'),
            "{name}: accessed {accessed:?} is not an RFC 3339 UTC time"
        );
        let sha = page.get("sha256");
        assert!(
            sha.len() == 64
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{name}: sha256 {sha:?}"
        );
        for key in ["bytes", "lines"] {
            let v: u64 = page
                .get(key)
                .parse()
                .unwrap_or_else(|_| panic!("{name}: {key}"));
            assert!(v > 0, "{name}: {key}");
        }
    }
}

/// One `kite:<page>.md[:<lines>]` citation.
struct Citation {
    line: usize,
    page: String,
    ranges: Vec<(u64, u64)>,
}

/// Every citation in `text`.
fn kite_citations(text: &str) -> Vec<Citation> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("kite:") {
            rest = &rest[at + 5..];
            let name_len = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c == '-' || c == '.'))
                .unwrap_or(rest.len());
            let name = rest[..name_len].trim_end_matches('.');
            if name.is_empty() {
                continue; // a placeholder such as `kite:<page>.md`
            }
            let mut ranges = Vec::new();
            let after = &rest[name.len()..];
            if let Some(spec) = after.strip_prefix(':') {
                let spec_len = spec
                    .find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '-'))
                    .unwrap_or(spec.len());
                for part in spec[..spec_len].split(',').filter(|p| !p.is_empty()) {
                    let (a, b) = part.split_once('-').unwrap_or((part, part));
                    let parse = |v: &str| {
                        v.parse::<u64>()
                            .unwrap_or_else(|_| panic!("line {}: bad range {part:?}", n + 1))
                    };
                    ranges.push((parse(a), parse(b)));
                }
            }
            out.push(Citation {
                line: n + 1,
                page: name.to_string(),
                ranges,
            });
        }
    }
    out
}

#[test]
fn every_kite_citation_resolves_to_a_recorded_page_and_lines() {
    let pages = kite_pages();
    let mut count = 0;
    let mut problems = Vec::new();
    for rel in scanned() {
        for Citation {
            line,
            page: name,
            ranges,
        } in kite_citations(&read(&rel))
        {
            count += 1;
            let Some(page) = pages.get(&name) else {
                problems.push(format!(
                    "{rel}:{line}: `kite:{name}` is not in docs/kite-sources.toml"
                ));
                continue;
            };
            let lines: u64 = page.get("lines").parse().unwrap();
            for (a, b) in ranges {
                if a == 0 || a > b || b > lines {
                    problems.push(format!(
                        "{rel}:{line}: `kite:{name}:{a}-{b}` outside 1..={lines}"
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(count > 100, "only {count} citations found");
}

#[test]
fn the_quota_profile_version_names_the_recorded_access_date() {
    let pages = kite_pages();
    let accessed = &pages["exceptions.md"].get("accessed")[..10];
    let source = read("src/kite/connect/admission.rs");
    let marker = "pub const QUOTA_PROFILE_VERSION: &str = \"";
    let start = source
        .find(marker)
        .expect("QUOTA_PROFILE_VERSION is defined")
        + marker.len();
    let version = &source[start..start + source[start..].find('"').unwrap()];
    // `<page>@<access date>`, optionally followed by `+r<revision>`.
    let prefix = format!("kite-connect-v3/exceptions.md@{accessed}");
    let revision = version
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("QUOTA_PROFILE_VERSION {version:?} does not name {prefix}"));
    assert!(
        revision.is_empty()
            || revision
                .strip_prefix("+r")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
        "QUOTA_PROFILE_VERSION {version:?} has a malformed revision"
    );
}

/// IDs of the forms `B-HTTP-01`, `B-TK-01`, `B-DEC-01`, `B-DIAG-01`,
/// `INV-<AREA>-01` and `BR-01` in `text`.
fn ids(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'-');
        if boundary && text.is_char_boundary(i) {
            for prefix in ["B-HTTP-", "B-TK-", "B-DEC-", "B-DIAG-", "BR-", "INV-"] {
                if !text[i..].starts_with(prefix) {
                    continue;
                }
                let mut j = i + prefix.len();
                if prefix == "INV-" {
                    let k = j + text[j..].bytes().take_while(u8::is_ascii_uppercase).count();
                    if k == j || bytes.get(k) != Some(&b'-') {
                        break;
                    }
                    j = k + 1;
                }
                let digits = text[j..].bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0 {
                    out.insert(text[i..j + digits].to_string());
                }
                break;
            }
        }
        i += 1;
    }
    out
}

/// IDs defined by the first cell of a table row in `docs/`.
fn defined_ids() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for rel in scanned()
        .iter()
        .filter(|r| r.starts_with("docs/") && r.ends_with(".md"))
    {
        for line in read(rel).lines() {
            if let Some(cell) = line.strip_prefix("| `")
                && let Some((id, _)) = cell.split_once('`')
            {
                out.extend(ids(id));
            }
        }
    }
    out
}

#[test]
fn every_used_id_is_defined_in_docs() {
    let defined = defined_ids();
    assert!(
        defined.contains("B-HTTP-01") && defined.contains("INV-B-01") && defined.contains("BR-01")
    );
    let mut used = 0;
    let mut problems = Vec::new();
    for rel in scanned() {
        for id in ids(&read(&rel)) {
            used += 1;
            if !defined.contains(&id) {
                problems.push(format!("{rel}: `{id}` is not defined in docs/"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(used > 100, "only {used} ID uses found");
}

/// Section numbers of the headings in a Markdown document.
fn sections(markdown: &str) -> BTreeSet<String> {
    markdown
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .filter_map(|l| {
            let number = l.trim_start_matches('#').trim_start().split(' ').next()?;
            let number = number.trim_end_matches('.');
            (!number.is_empty() && number.split('.').all(|p| p.parse::<u32>().is_ok()))
                .then(|| number.to_string())
        })
        .collect()
}

#[test]
fn every_section_reference_names_an_existing_section() {
    let docs = ["contract.md", "verification.md", "migration.md"];
    let known: BTreeMap<&str, BTreeSet<String>> = docs
        .iter()
        .map(|d| (*d, sections(&read(&format!("docs/{d}")))))
        .collect();
    let mut count = 0;
    let mut problems = Vec::new();
    // This file names `§` in its own code, so it is not scanned.
    for rel in scanned().into_iter().filter(|r| r != "tests/references.rs") {
        let text = read(&rel);
        let own = rel.strip_prefix("docs/").filter(|d| docs.contains(d));
        for (at, _) in text.match_indices('§') {
            let number: String = text[at + '§'.len_utf8()..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let number = number.trim_end_matches('.');
            if number.is_empty() {
                continue; // a placeholder such as `§N`
            }
            let before = text[..at].trim_end_matches([' ', '`']);
            let target = docs.iter().find(|d| before.ends_with(**d)).copied().or(own);
            let line = text[..at].lines().count();
            count += 1;
            match target {
                Some(doc) if known[doc].contains(number) => {}
                Some(doc) => problems.push(format!("{rel}:{line}: {doc} has no §{number}")),
                None => problems.push(format!("{rel}:{line}: §{number} names no document")),
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(count > 20, "only {count} section references found");
}

#[test]
fn no_file_refers_to_material_outside_the_repository() {
    // Assembled at run time so this file does not match itself.
    let forbidden: Vec<String> = [
        ["plan", " task"],
        ["arch", " §"],
        ["architecture", " §"],
        ["SDK contract", " §"],
        ["sdk-", "contract"],
        ["decompo", "sition"],
        ["kite-api", "-docs"],
        [".tra", "cker"],
        [".bea", "ds"],
        ["Bea", "ds"],
        ["AUX", "-mv1"],
        ["DEC", "-Q"],
        ["FX", "-01"],
        ["G", "-01"],
    ]
    .iter()
    .map(|[a, b]| format!("{a}{b}"))
    .collect();
    let mut problems = Vec::new();
    for rel in scanned() {
        for (n, line) in read(&rel).lines().enumerate() {
            for term in &forbidden {
                // A term counts only at a word start, not inside a longer ID.
                let hit = line.match_indices(term.as_str()).any(|(at, _)| {
                    !line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
                });
                if hit {
                    problems.push(format!("{rel}:{}: {term:?}", n + 1));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
