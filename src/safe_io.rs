//! Atomic, symlink-safe file writes for include write-back.
//!
//! Writable includes (`dump_includes`, `YAMLRocksDocument.save`) re-write source
//! files that were recorded when the document was loaded. A plain
//! `std::fs::write` follows a symlink at the target path, so an attacker who can
//! swap a tracked source file for a symlink between load and write-back could
//! redirect the write to a file outside the configuration tree (a narrow but real
//! clobber-outside-the-tree vector when a privileged process writes back a tree
//! an attacker can stage).
//!
//! [`atomic_write`] closes this: it writes the new content to a fresh temporary
//! file in the same directory, created with `O_CREAT | O_EXCL` (which never
//! follows a symlink), then renames it over the target. `rename` replaces the
//! path itself rather than following it, so a symlink swapped in at the target is
//! overwritten, not traversed, and the write stays in the tree. The rename is
//! also atomic, so a concurrent reader sees either the old file or the new one,
//! never a half-written one.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-process counter making each temporary file name unique without randomness.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically and without following a symlink at `path`.
///
/// Writes to a temporary sibling file and renames it over `path`. See the module
/// docs for why this is the safe write for include write-back.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        // A bare file name (no parent) is written relative to the current
        // directory, so the temporary file belongs there too.
        _ => Path::new("."),
    };
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot write a path that ends in `..` or is empty",
        )
    })?;

    // Create a uniquely named temporary file in the target's directory. `O_EXCL`
    // (via `create_new`) both guarantees the name is unused and refuses to follow
    // a symlink, so the staged content is never written through one. The temp
    // must share the directory (and thus the filesystem) so the later rename is a
    // cheap atomic metadata operation, not a cross-device copy.
    let pid = std::process::id();
    let (tmp_path, mut file) = loop {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut name = file_name.to_os_string();
        name.push(format!(".yamlrocks-tmp.{pid}.{counter}"));
        let candidate: PathBuf = dir.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };

    // Write the content, then move it into place. On any failure remove the
    // temporary file so a partial write never lingers next to the real one.
    let staged = file.write_all(bytes).and_then(|()| file.flush());
    drop(file);
    if let Err(e) = staged.and_then(|()| std::fs::rename(&tmp_path, path)) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::atomic_write;
    use std::fs;

    /// A fresh write lands the exact bytes at the path.
    #[test]
    fn writes_new_file() {
        let dir = std::env::temp_dir().join(format!("yamlrocks-safeio-new-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.yaml");
        atomic_write(&target, b"key: value\n").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"key: value\n");
        fs::remove_dir_all(&dir).ok();
    }

    /// Overwriting a symlinked target replaces the link with a real file instead
    /// of writing through it, so the link's target is left untouched.
    #[test]
    #[cfg(unix)]
    fn replaces_symlink_instead_of_following_it() {
        let dir =
            std::env::temp_dir().join(format!("yamlrocks-safeio-link-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let outside = dir.join("outside.txt");
        fs::write(&outside, b"original\n").unwrap();
        let target = dir.join("link.yaml");
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        atomic_write(&target, b"new: data\n").unwrap();

        // The link target was not clobbered, and the path is now a real file.
        assert_eq!(fs::read(&outside).unwrap(), b"original\n");
        assert!(!fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"new: data\n");
        fs::remove_dir_all(&dir).ok();
    }
}
