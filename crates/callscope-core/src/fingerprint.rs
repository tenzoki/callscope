//! Workspace source fingerprint and staleness check (Q5 repeatable, Q6
//! detectable staleness).
//!
//! One implementation, used by two crates: `callscope-index` calls
//! [`fingerprint_workspace`] to write `manifest.json` alongside the index, and
//! `callscope-mcp` calls [`diverged_files`] on every request to tell whether the
//! index still matches the sources on disk. There is deliberately no second
//! copy of this logic — the per-file hashing is factored into one private
//! helper ([`hash_all_rs`]) that both entry points share, so the two crates can
//! never drift on what "the same workspace state" means.
//!
//! # Hash choice: FNV-1a, 64-bit
//!
//! Every content hash here is FNV-1a, the same algorithm
//! [`crate::schema::SymbolId::from_fq_path`] already uses for content-addressing
//! symbol ids. Three reasons, in order of weight:
//!
//! 1. **The codebase already speaks FNV-1a.** Reusing it keeps callscope on one
//!    hashing algorithm rather than introducing a second one for the same job
//!    (content-addressing for change detection). The stability argument written
//!    out in `schema.rs` applies verbatim: FNV-1a is a fixed, dependency-free
//!    algorithm, so the same bytes hash the same on every platform and every
//!    toolchain — unlike the standard library's SipHash, whose output is not
//!    guaranteed stable across Rust versions. A fingerprint that drifted under a
//!    toolchain bump would report false staleness on unchanged code.
//! 2. **No new dependency.** `callscope-core` is the stable shared foundation;
//!    keeping its dependency surface minimal is worth more here than the
//!    marginal collision resistance a crate like `blake3`/`sha2`/`seahash` would
//!    add.
//! 3. **Cryptographic strength is not required.** This is change *detection*,
//!    not tamper-proofing. A collision would mean an edit to a file produced the
//!    identical 64-bit hash as its previous content — at ~1 in 1.8e19 per edit,
//!    negligible for a workspace of the size callscope targets.
//!
//! FNV-1a's constants are restated here rather than shared with `schema.rs`
//! because the scope of this task forbids editing `schema.rs`. That duplication
//! is tracked as a follow-up (consolidate into one `fnv1a_64` helper when the
//! schema module can be touched); the constants are the fixed FNV-1a standard,
//! so the two copies cannot silently disagree.
//!
//! # Staleness strategy: hash-all, not mtime-first
//!
//! The plan sketched a mtime-first fast path ("stat mtimes, hash only changed
//! files"). That path needs a *stored* mtime per file to compare the current
//! mtime against — and the [`Manifest`] schema (fixed by an earlier task, not
//! editable here) records content hashes only, no mtimes. With nothing to
//! compare a current mtime to, a stat-first check gains nothing. So
//! [`diverged_files`] re-hashes every `.rs` file and diffs the result against
//! the stored hashes. For a workspace of callscope's scale this is O(total
//! source bytes) and trivially fast, and it is *exact*: it never misses an edit
//! that a mtime-only check would (a `touch` with no edit, or an edit landing
//! within the filesystem's mtime granularity). If re-index cost ever matters at
//! a much larger scale, the manifest could grow per-file mtimes to enable the
//! stat-first path — that is a schema change, deliberately out of scope here.

use crate::schema::{Manifest, SCHEMA_VERSION};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Compute the full source fingerprint of the workspace rooted at
/// `workspace_root`.
///
/// Walks the tree for every `.rs` file (see [`hash_all_rs`] for what is
/// included), hashes each, hashes `Cargo.lock`, and stamps the toolchain and
/// build time. `toolchain` is supplied by the caller because only the caller
/// (`callscope-index`) knows the exact nightly it linked; the manifest records
/// it verbatim so a later run can tell whether the toolchain itself changed.
///
/// The result is deterministic for identical workspace content except for
/// `indexed_at`, which is the wall-clock build time by design.
pub fn fingerprint_workspace(workspace_root: &Path, toolchain: &str) -> io::Result<Manifest> {
    let file_hashes = hash_all_rs(workspace_root)?;
    let cargo_lock_hash = hash_cargo_lock(workspace_root)?;
    Ok(Manifest {
        schema_version: SCHEMA_VERSION,
        toolchain: toolchain.to_string(),
        file_hashes,
        cargo_lock_hash,
        indexed_at: now_rfc3339(),
    })
}

/// Cheap staleness check: which indexed inputs diverged from `manifest`.
///
/// Returns workspace-relative paths, sorted, covering three kinds of divergence:
/// a `.rs` file whose content hash changed, a `.rs` file added since the
/// manifest was written, and a `.rs` file removed since. `Cargo.lock` is
/// included under that name when its hash changed, so a dependency bump that can
/// alter the resolved call graph also shows up (Q6).
///
/// An empty result means the index is up to date. The caller wraps the result
/// in [`crate::envelope::StaleInfo`] and attaches it to the envelope only when
/// the vector is non-empty.
///
/// Toolchain divergence is intentionally *not* folded in here: [`StaleInfo`]
/// carries a file list, and the toolchain is not a file. A caller that cares
/// compares `manifest.toolchain` to the current toolchain itself.
///
/// [`StaleInfo`]: crate::envelope::StaleInfo
pub fn diverged_files(manifest: &Manifest, workspace_root: &Path) -> io::Result<Vec<String>> {
    let current = hash_all_rs(workspace_root)?;
    let mut diverged: Vec<String> = Vec::new();

    // Changed or added: present now, but absent-or-different in the manifest.
    for (path, hash) in &current {
        if manifest.file_hashes.get(path) != Some(hash) {
            diverged.push(path.clone());
        }
    }
    // Removed: recorded in the manifest, gone from disk now.
    for path in manifest.file_hashes.keys() {
        if !current.contains_key(path) {
            diverged.push(path.clone());
        }
    }
    // Dependency drift: Cargo.lock content changed.
    if hash_cargo_lock(workspace_root)? != manifest.cargo_lock_hash {
        diverged.push("Cargo.lock".to_string());
    }

    diverged.sort();
    diverged.dedup();
    Ok(diverged)
}

/// Hash every `.rs` file under `root`, keyed by workspace-relative path.
///
/// The single source of truth for "the source state of a workspace", shared by
/// [`fingerprint_workspace`] and [`diverged_files`]. Skips `target/`, `.git/`,
/// and any dotted directory, so build output and VCS metadata never enter the
/// fingerprint. Symlinks are ignored (the entry's file type is read without
/// following the link, so a symlinked directory is neither a dir nor a file
/// here) — that keeps the walk free of cycles and of paths outside the tree.
///
/// Keys are normalised to `/` separators regardless of platform, so the same
/// workspace fingerprints identically on Windows and Unix.
fn hash_all_rs(root: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    collect_rs(root, root, &mut out)?;
    Ok(out)
}

fn collect_rs(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_rs(&entry.path(), root, out)?;
        } else if file_type.is_file() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let bytes = std::fs::read(&path)?;
                out.insert(relative_key(root, &path), fnv1a_64_hex(&bytes));
            }
        }
        // Symlinks and other node types are ignored by design.
    }
    Ok(())
}

/// Hash `<root>/Cargo.lock`. A missing lockfile yields the empty string (a
/// stable sentinel: two runs both without a lockfile compare equal and so
/// report no staleness). Any I/O error other than "not found" propagates.
fn hash_cargo_lock(root: &Path) -> io::Result<String> {
    match std::fs::read(root.join("Cargo.lock")) {
        Ok(bytes) => Ok(fnv1a_64_hex(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Workspace-relative path with `/` separators, for a deterministic, portable
/// map key. Falls back to the absolute path if `path` is somehow not under
/// `root` (should not happen for files the walk produced).
fn relative_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// FNV-1a over raw bytes, rendered as 16 lowercase hex digits. See the module
/// docs for why this algorithm. The constants are the fixed FNV-1a 64-bit
/// standard, matching [`crate::schema::SymbolId::from_fq_path`].
fn fnv1a_64_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// Current wall-clock time as an RFC-3339 UTC string (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// Rolled by hand rather than pulling in a date crate: `indexed_at` is
/// informational (it records when the index was built), the staleness check
/// never reads it, and keeping `callscope-core` free of a datetime dependency is
/// worth the small conversion below.
fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    unix_secs_to_rfc3339(secs)
}

/// Convert whole seconds since the Unix epoch to an RFC-3339 UTC string.
///
/// Uses Howard Hinnant's `civil_from_days` algorithm, which is exact for all
/// dates (leap years and centuries included) with no lookup tables.
fn unix_secs_to_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // civil_from_days: days since 1970-01-01 -> (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = year + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A std-only scratch directory that removes itself on drop. Avoids adding a
    /// `tempfile` dev-dependency, so the tests run offline and the shared core
    /// crate gains no dependency at all for this task.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "callscope-fp-{tag}-{}-{nanos}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir { path }
        }

        fn write(&self, rel: &str, content: &str) {
            let full = self.path.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(full, content).expect("write file");
        }

        fn remove(&self, rel: &str) {
            std::fs::remove_file(self.path.join(rel)).expect("remove file");
        }

        fn root(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn sample_workspace(tag: &str) -> TempDir {
        let dir = TempDir::new(tag);
        dir.write("src/a.rs", "fn a() {}\n");
        dir.write("src/b.rs", "fn b() -> u32 { 0 }\n");
        dir.write("Cargo.lock", "# lockfile v1\n");
        dir
    }

    #[test]
    fn fingerprint_is_stable_across_runs_on_identical_content() {
        let dir = sample_workspace("stable");
        let first = fingerprint_workspace(dir.root(), "nightly-2026-07-26").unwrap();
        let second = fingerprint_workspace(dir.root(), "nightly-2026-07-26").unwrap();

        // Everything the fingerprint content-addresses is identical; only the
        // wall-clock stamp may differ between the two runs.
        assert_eq!(first.file_hashes, second.file_hashes);
        assert_eq!(first.cargo_lock_hash, second.cargo_lock_hash);
        assert_eq!(first.toolchain, second.toolchain);
        assert_eq!(
            first.file_hashes.keys().collect::<Vec<_>>(),
            vec![&"src/a.rs".to_string(), &"src/b.rs".to_string()],
        );
    }

    #[test]
    fn identical_workspace_reports_no_divergence() {
        let dir = sample_workspace("nodiverge");
        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();
        assert!(diverged_files(&manifest, dir.root()).unwrap().is_empty());
    }

    #[test]
    fn changed_content_flips_hash_and_reports_exactly_that_file() {
        let dir = sample_workspace("changed");
        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();
        let before = manifest.file_hashes["src/a.rs"].clone();

        dir.write("src/a.rs", "fn a() { let _ = 1 + 1; }\n");

        // The hash of the edited file flipped.
        let refreshed = fingerprint_workspace(dir.root(), "tc").unwrap();
        assert_ne!(before, refreshed.file_hashes["src/a.rs"]);
        // b.rs was untouched.
        assert_eq!(manifest.file_hashes["src/b.rs"], refreshed.file_hashes["src/b.rs"]);

        // Staleness reports exactly the changed file, nothing else.
        assert_eq!(
            diverged_files(&manifest, dir.root()).unwrap(),
            vec!["src/a.rs".to_string()],
        );
    }

    #[test]
    fn added_file_is_detected() {
        let dir = sample_workspace("added");
        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();

        dir.write("src/c.rs", "fn c() {}\n");

        assert_eq!(
            diverged_files(&manifest, dir.root()).unwrap(),
            vec!["src/c.rs".to_string()],
        );
    }

    #[test]
    fn removed_file_is_detected() {
        let dir = sample_workspace("removed");
        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();

        dir.remove("src/b.rs");

        assert_eq!(
            diverged_files(&manifest, dir.root()).unwrap(),
            vec!["src/b.rs".to_string()],
        );
    }

    #[test]
    fn cargo_lock_change_is_detected() {
        let dir = sample_workspace("lock");
        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();

        dir.write("Cargo.lock", "# lockfile v1\n# new dependency pinned\n");

        assert_eq!(
            diverged_files(&manifest, dir.root()).unwrap(),
            vec!["Cargo.lock".to_string()],
        );
    }

    #[test]
    fn target_and_hidden_dirs_are_excluded() {
        let dir = TempDir::new("exclude");
        dir.write("src/lib.rs", "fn real() {}\n");
        dir.write("target/debug/generated.rs", "fn build_output() {}\n");
        dir.write(".hidden/secret.rs", "fn hidden() {}\n");

        let manifest = fingerprint_workspace(dir.root(), "tc").unwrap();
        assert_eq!(
            manifest.file_hashes.keys().collect::<Vec<_>>(),
            vec![&"src/lib.rs".to_string()],
            "only source under the workspace, not build output or dotted dirs",
        );
    }

    #[test]
    fn distinct_content_hashes_differ() {
        assert_ne!(fnv1a_64_hex(b"fn a() {}"), fnv1a_64_hex(b"fn b() {}"));
        // Identical bytes hash identically (repeatability at the primitive level).
        assert_eq!(fnv1a_64_hex(b"same"), fnv1a_64_hex(b"same"));
        // Rendered width is always 16 hex digits.
        assert_eq!(fnv1a_64_hex(b"").len(), 16);
    }

    #[test]
    fn rfc3339_conversion_matches_known_epochs() {
        assert_eq!(unix_secs_to_rfc3339(0), "1970-01-01T00:00:00Z");
        // 2021-01-01T00:00:00Z is a well-known epoch value.
        assert_eq!(unix_secs_to_rfc3339(1_609_459_200), "2021-01-01T00:00:00Z");
        // A time-of-day check that also crosses into a leap year (2020).
        assert_eq!(unix_secs_to_rfc3339(1_583_020_800), "2020-03-01T00:00:00Z");
    }
}
