//! Atomic file writes: temp-file + fsync + rename + parent-dir-fsync
//! (LLD-01 §6.2). The only path any Rust caller uses to write a user
//! artifact — no `File::create` + `write_all` scattered elsewhere.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::error::AppError;

/// Writes `bytes` to `path` such that `path` either holds its previous
/// contents or the new ones in full — never a partial write, even if the
/// process is killed mid-call.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::storage("atomic_write: path has no parent"))?;
    fs::create_dir_all(parent)?;

    // Write to a per-call-unique temp name so concurrent writers to the same
    // logical path race the rename, not the write.
    let tmp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("blob"),
        std::process::id(),
        uuid::Uuid::new_v4().simple(),
    ));

    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp); // best-effort cleanup
        return Err(e.into());
    }

    // fsync the parent dir so the rename itself is durable (POSIX subtlety;
    // Windows NTFS/ReFS commit ordering handles this without an equivalent).
    #[cfg(unix)]
    {
        let dir = fs::File::open(parent)?;
        dir.sync_all()?;
    }

    Ok(())
}

pub fn atomic_write_json<T: Serialize>(path: &Path, val: &T) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(val)
        .map_err(|e| AppError::storage(format!("json serialise: {e}")))?;
    atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn round_trips_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.txt");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn overwrite_replaces_contents_wholesale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second, and longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, and longer");
    }

    #[test]
    fn no_temp_files_left_behind_after_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        atomic_write(&path, b"hello").unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty());
    }

    /// Durability under a simulated crash between write-and-rename: an
    /// abandoned temp file (as if the process died before the rename ran)
    /// must never corrupt or replace the real target on a subsequent write.
    #[test]
    fn abandoned_tmp_file_never_corrupts_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        atomic_write(&path, b"original").unwrap();

        // Simulate a crash: leave a stray tmp file with partial/garbage
        // content, as `atomic_write` would if killed after opening the tmp
        // file but before the rename.
        let stray = dir.path().join(".file.txt.tmp.99999.deadbeef");
        fs::write(&stray, b"PARTIAL-GARBAGE").unwrap();

        // Target is untouched — never a corrupt merge, never the stray data.
        assert_eq!(fs::read(&path).unwrap(), b"original");

        // A subsequent legitimate write still succeeds and is clean.
        atomic_write(&path, b"recovered").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"recovered");

        // The stray file from the "crashed" attempt is inert leftover, not
        // consulted by atomic_write — a real boot-time sweep (out of scope
        // here) would remove it, but its mere presence must never corrupt.
        assert!(stray.exists());
        assert_eq!(fs::read(&stray).unwrap(), b"PARTIAL-GARBAGE");
    }

    /// Concurrent writers to the same path: both winners produce a valid,
    /// uncorrupted file — the final content is one writer's bytes in full,
    /// never an interleaving of both.
    #[test]
    fn concurrent_writers_never_interleave() {
        let dir = Arc::new(tempfile::tempdir().unwrap());
        let path = dir.path().join("file.txt");
        atomic_write(&path, b"seed").unwrap();

        let candidates = ["A".repeat(5_000), "B".repeat(7_000)];
        let handles: Vec<_> = candidates
            .iter()
            .cloned()
            .map(|payload| {
                let dir = Arc::clone(&dir);
                thread::spawn(move || {
                    let path = dir.path().join("file.txt");
                    atomic_write(&path, payload.as_bytes()).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let final_bytes = fs::read(&path).unwrap();
        let final_str = String::from_utf8(final_bytes).unwrap();
        assert!(
            candidates.contains(&final_str),
            "final content must be exactly one writer's payload, got {} bytes",
            final_str.len()
        );
    }

    #[test]
    fn json_helper_round_trips() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Doc {
            n: i64,
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        atomic_write_json(&path, &Doc { n: 42 }).unwrap();
        let back: Doc = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(back, Doc { n: 42 });
    }
}
