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

const MAGIC: &[u8; 8] = b"AGNOFP02";

/// Stable 64-bit FNV-1a. Deliberately not `DefaultHasher`: fingerprints must
/// survive toolchain upgrades, otherwise every Rust update costs a full build.
#[must_use]
pub fn fingerprint(seed: u64, bytes: &[u8]) -> u64 {
    let mut hash = seed ^ 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[must_use]
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

/// One page's cached render fingerprint and summary.
///
/// Private: no public method hands an `Entry` out, so making the type public
/// only promised an API the crate does not keep.
struct Entry {
    key: u64,
    summary: String,
}

pub struct Fingerprints {
    header: u64,
    entries: HashMap<String, Entry>,
}

impl Fingerprints {
    #[must_use]
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
    #[must_use]
    pub fn summary(&self, url: &str, key: u64) -> Option<&str> {
        self.entries
            .get(url)
            .filter(|entry| entry.key == key)
            .map(|entry| entry.summary.as_str())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Write the fingerprint file, replacing the previous one in one rename.
    ///
    /// # Errors
    ///
    /// Fails when the cache directory cannot be created, the file cannot be
    /// written or renamed, or a stored string is over the 4 GiB field limit.
    pub fn save<'a>(
        &self,
        root: &Path,
        items: impl Iterator<Item = (&'a str, u64, &'a str)>,
    ) -> std::io::Result<()> {
        let items: Vec<(&str, u64, &str)> = items.collect();
        let mut out = Vec::with_capacity(16 + items.len() * 64);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.header.to_le_bytes());
        out.extend_from_slice(&field_len(items.len(), "the page count")?.to_le_bytes());
        for (url, key, summary) in items {
            let url = url.as_bytes();
            let summary = summary.as_bytes();
            // Lengths are u32 and never clamped: a frontmatter summary is
            // arbitrary text, and truncating it here made a rebuild render a
            // shorter summary than the build that filled the cache.
            out.extend_from_slice(&field_len(url.len(), "a cached url")?.to_le_bytes());
            out.extend_from_slice(url);
            out.extend_from_slice(&key.to_le_bytes());
            out.extend_from_slice(&field_len(summary.len(), "a cached summary")?.to_le_bytes());
            out.extend_from_slice(summary);
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

/// A length that fits the on-disk `u32` field.
///
/// The previous version wrote `len as u32`, which wraps silently: a 4 GiB
/// summary would be stored as a short one and the rebuild would render
/// different output than the build that filled the cache. Refusing to write is
/// the only honest option — the cache is an optimisation, so a too-large value
/// can fall back to a full build.
fn field_len(len: usize, what: &str) -> std::io::Result<u32> {
    u32::try_from(len).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{what} is {len} bytes, over the 4 GiB cache limit"),
        )
    })
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
        let url_len = read_u32(bytes, &mut cursor)?;
        let url = std::str::from_utf8(bytes.get(cursor..cursor.checked_add(url_len)?)?).ok()?;
        cursor += url_len;
        let key = u64::from_le_bytes(bytes.get(cursor..cursor + 8)?.try_into().ok()?);
        cursor += 8;
        let summary_len = read_u32(bytes, &mut cursor)?;
        let summary =
            std::str::from_utf8(bytes.get(cursor..cursor.checked_add(summary_len)?)?).ok()?;
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

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
    let value = u32::from_le_bytes(bytes.get(*cursor..*cursor + 4)?.try_into().ok()?);
    *cursor += 4;
    usize::try_from(value).ok()
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

    #[test]
    fn round_trips_a_summary_larger_than_64_kib() {
        let root = std::env::temp_dir().join(format!("agno-cache-big-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let summary = "ş".repeat(100_000);
        let cache = Fingerprints::load(&root, 1);
        cache
            .save(&root, [("/a/", 7, summary.as_str())].into_iter())
            .unwrap();

        let loaded = Fingerprints::load(&root, 1);
        assert_eq!(
            loaded.summary("/a/", 7).map(str::len),
            Some(summary.len()),
            "a long summary must survive the round trip uncut"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
