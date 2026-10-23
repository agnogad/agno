//! Per-page render fingerprints for incremental builds.
//!
//! A build is deterministic for a page when three things stay the same: the
//! page source, the templates, and the site-wide context (`site.*`). The
//! fingerprint file stores a hash of the first and a header hash of the other
//! two, so a single edit only re-renders the page that changed. Rendered HTML
//! is not duplicated here: the existing file in `dist/` is the cache, and it
//! is reused only when its fingerprint still matches.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"AGNOFP01";

/// Stable 64-bit FNV-1a. Deliberately not `DefaultHasher`: fingerprints must
/// survive toolchain upgrades, otherwise every Rust update costs a full build.
pub fn fingerprint(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed ^ 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn combine(values: &[u64]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for value in values {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

pub struct Entry {
    pub key: u64,
    pub summary: String,
}

pub struct Fingerprints {
    header: u64,
    entries: HashMap<String, Entry>,
}

impl Fingerprints {
    pub fn load(root: &Path, header: u64) -> Self {
        let empty = Self {
            header,
            entries: HashMap::new(),
        };
        let Ok(bytes) = fs::read(path(root)) else {
            return empty;
        };
        match decode(&bytes, header) {
            Some(entries) => Self { header, entries },
            None => empty,
        }
    }

    /// The stored summary for `url`, but only when the page source is unchanged.
    pub fn summary(&self, url: &str, key: u64) -> Option<&str> {
        self.entries
            .get(url)
            .filter(|entry| entry.key == key)
            .map(|entry| entry.summary.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn save<'a>(
        &self,
        root: &Path,
        items: impl Iterator<Item = (&'a str, u64, &'a str)>,
    ) -> std::io::Result<()> {
        let items: Vec<(&str, u64, &str)> = items.collect();
        let mut out = Vec::with_capacity(16 + items.len() * 64);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.header.to_le_bytes());
        out.extend_from_slice(&(items.len() as u32).to_le_bytes());
        for (url, key, summary) in items {
            let url = url.as_bytes();
            let summary = summary.as_bytes();
            let url_len = u16::try_from(url.len()).unwrap_or(u16::MAX);
            let summary_len = u16::try_from(summary.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&url_len.to_le_bytes());
            out.extend_from_slice(&url[..usize::from(url_len)]);
            out.extend_from_slice(&key.to_le_bytes());
            out.extend_from_slice(&summary_len.to_le_bytes());
            out.extend_from_slice(&summary[..usize::from(summary_len)]);
        }

        let target = path(root);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = target.with_extension("tmp");
        fs::write(&tmp, &out)?;
        fs::rename(&tmp, &target)
    }
}

fn path(root: &Path) -> PathBuf {
    root.join(".agno-cache").join("fingerprints.bin")
}

fn decode(bytes: &[u8], header: u64) -> Option<HashMap<String, Entry>> {
    if bytes.len() < 20 || &bytes[..8] != MAGIC {
        return None;
    }
    let stored = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    if stored != header {
        return None;
    }
    let count = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
    let mut entries = HashMap::with_capacity(count.min(1 << 20));
    let mut cursor = 20usize;
    for _ in 0..count {
        let url_len = read_u16(bytes, &mut cursor)?;
        let url = std::str::from_utf8(bytes.get(cursor..cursor + url_len)?).ok()?;
        cursor += url_len;
        let key = u64::from_le_bytes(bytes.get(cursor..cursor + 8)?.try_into().ok()?);
        cursor += 8;
        let summary_len = read_u16(bytes, &mut cursor)?;
        let summary = std::str::from_utf8(bytes.get(cursor..cursor + summary_len)?).ok()?;
        cursor += summary_len;
        entries.insert(
            url.to_string(),
            Entry {
                key,
                summary: summary.to_string(),
            },
        );
    }
    Some(entries)
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
    let value = u16::from_le_bytes(bytes.get(*cursor..*cursor + 2)?.try_into().ok()?);
    *cursor += 2;
    Some(usize::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_change_with_content() {
        assert_ne!(fingerprint(0, b"a"), fingerprint(0, b"b"));
        assert_eq!(fingerprint(0, b"a"), fingerprint(0, b"a"));
    }

    #[test]
    fn round_trips_entries_and_drops_stale_headers() {
        let root = std::env::temp_dir().join(format!("agno-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let cache = Fingerprints::load(&root, 42);
        cache
            .save(
                &root,
                [("/a/", 7, "hello"), ("/b/", 9, "world")].into_iter(),
            )
            .unwrap();

        let loaded = Fingerprints::load(&root, 42);
        assert_eq!(loaded.summary("/a/", 7), Some("hello"));
        assert_eq!(loaded.summary("/b/", 8), None, "key mismatch is a miss");

        let stale = Fingerprints::load(&root, 43);
        assert!(stale.is_empty(), "header change invalidates the cache");

        let _ = fs::remove_dir_all(&root);
    }
}
