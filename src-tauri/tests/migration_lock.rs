//! Guards `db::LOCKED_MIGRATIONS`: once a migration filename is added to
//! that list, this test fails the moment `src/db/migrations/<filename>`'s
//! content changes, catching the same "schema drift" sqlx would otherwise
//! only discover against a real user's already-applied DB (see the const's
//! doc comment in `src/db/mod.rs`).
//!
//! Hashing (not a byte-for-byte fixture copy) because a locked file's
//! expected value is a one-line constant here rather than a second checked-in
//! copy of the file to keep in sync. SHA-256 via `sha2` (already a
//! dependency) rather than `DefaultHasher`, whose output isn't guaranteed
//! stable across Rust versions.

use std::path::Path;

use mnemos_tauri_lib::db::LOCKED_MIGRATIONS;
use sha2::{Digest, Sha256};

fn hash_file(path: &Path) -> String {
    let content = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let digest = Sha256::digest(&content);
    format!("{digest:x}")
}

/// Locked filename -> expected SHA-256 hex digest of its content, recorded
/// the moment the filename was added to `LOCKED_MIGRATIONS`. Add an entry
/// here in the same commit that adds the filename to the const.
const EXPECTED_HASHES: &[(&str, &str)] = &[(
    "001_init.sql",
    "7f4a86083736574027603ee6f5406e3eff3eef10183ef42816f87138aa17ca46",
)];

#[test]
fn locked_migrations_are_unmodified() {
    let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/db/migrations");

    for &filename in LOCKED_MIGRATIONS {
        let expected = EXPECTED_HASHES
            .iter()
            .find(|(name, _)| *name == filename)
            .unwrap_or_else(|| {
                panic!(
                    "{filename} is in LOCKED_MIGRATIONS but has no entry in \
                     EXPECTED_HASHES in tests/migration_lock.rs — add one \
                     when locking a migration"
                )
            })
            .1;

        let actual = hash_file(&migrations_dir.join(filename));
        assert_eq!(
            actual, expected,
            "{filename} is locked (see LOCKED_MIGRATIONS in db/mod.rs) but its \
             content changed. A locked migration must never be edited — add a \
             new numbered migration file instead."
        );
    }
}

/// Proves the comparison mechanism itself detects a content change,
/// independent of what is currently locked — `locked_migrations_are_unmodified`
/// would still pass vacuously if `LOCKED_MIGRATIONS` were ever emptied.
#[test]
fn hash_mismatch_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("001_init.sql");

    std::fs::write(&file, b"-- original content\nCREATE TABLE t (id TEXT);\n").unwrap();
    let recorded = hash_file(&file);

    // Unmodified: hash matches what was recorded when "locked".
    assert_eq!(hash_file(&file), recorded);

    // Modified (as an unrelated cleanup pass editing a comment would do):
    // hash must no longer match.
    std::fs::write(&file, b"-- edited content\nCREATE TABLE t (id TEXT);\n").unwrap();
    assert_ne!(hash_file(&file), recorded);
}
