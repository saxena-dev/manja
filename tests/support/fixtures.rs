// Test fixture loading.
//
// Every path is resolved from this crate's `CARGO_MANIFEST_DIR`, captured at
// compile time, and never from the process working directory, so a test
// binary finds its fixtures however and wherever it is launched. A missing or
// malformed fixture is an error naming the resolved path. Nothing here
// downloads, skips, or substitutes a synthetic fixture.
//
// This file is shared by the library's unit tests (pulled into
// `src/kite/connect/client.rs` with `include!`) and by the integration test
// support crate, so it holds only items, has no inner attributes, and depends
// only on `std`, `serde` and `serde_json`.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// Directory holding the official, upstream `kiteconnect-mocks` fixtures.
pub fn mocks_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("kiteconnect-mocks")
}

/// Why a fixture could not be loaded. Each variant carries the resolved path.
pub enum FixtureError {
    /// The fixture directory itself is absent, e.g. the `kiteconnect-mocks`
    /// submodule was never initialized. A setup failure, not a skipped test.
    MissingDir(PathBuf),
    /// The fixture file could not be read.
    Unreadable(PathBuf, std::io::Error),
    /// The fixture file was read but is not valid JSON for the requested type.
    Malformed(PathBuf, serde_json::Error),
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDir(dir) => write!(
                f,
                "fixture directory {} does not exist; initialize the kiteconnect-mocks \
                 submodule (tests never download or synthesize fixtures)",
                dir.display()
            ),
            Self::Unreadable(path, e) => {
                write!(f, "cannot read fixture {}: {e}", path.display())
            }
            Self::Malformed(path, e) => {
                write!(f, "malformed fixture {}: {e}", path.display())
            }
        }
    }
}

// `Debug` delegates to `Display` so that `.unwrap()` on a fixture result
// prints the actionable message, path included.
impl fmt::Debug for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for FixtureError {}

/// Read fixture `name` from `dir` as UTF-8 text.
pub fn read_in(dir: &Path, name: &str) -> Result<String, FixtureError> {
    if !dir.is_dir() {
        return Err(FixtureError::MissingDir(dir.to_path_buf()));
    }
    let path = dir.join(name);
    std::fs::read_to_string(&path).map_err(|e| FixtureError::Unreadable(path, e))
}

/// Deserialize fixture `name` from `dir` as JSON.
pub fn json_in<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<T, FixtureError> {
    let text = read_in(dir, name)?;
    serde_json::from_str(&text).map_err(|e| FixtureError::Malformed(dir.join(name), e))
}

/// Read official fixture `name` as UTF-8 text, byte-for-byte unchanged.
pub fn read(name: &str) -> Result<String, FixtureError> {
    read_in(&mocks_dir(), name)
}

/// Deserialize official fixture `name` as JSON.
pub fn json<T: DeserializeOwned>(name: &str) -> Result<T, FixtureError> {
    json_in(&mocks_dir(), name)
}

/// Read official JSON fixture `name` unchanged, for serving as a response body.
///
/// The text is checked to parse as JSON first, so a corrupted fixture fails
/// here, naming its path, rather than later as an anonymous client-side
/// deserialization error.
pub fn json_body(name: &str) -> Result<String, FixtureError> {
    let text = read(name)?;
    serde_json::from_str::<serde::de::IgnoredAny>(&text)
        .map_err(|e| FixtureError::Malformed(mocks_dir().join(name), e))?;
    Ok(text)
}
