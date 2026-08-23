//! Same-directory atomic replacement shared by captured fixtures and rollback state.
//! Destinations are never opened for writing, and temporary files use `create_new`, so
//! pre-existing symlinks and hardlinks cannot be followed or truncated.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use eyre::{bail, eyre, Context, Result};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temp_path(destination: &Path, label: &str, id: u64) -> Result<PathBuf> {
    let parent = destination.parent().ok_or_else(|| {
        eyre!(
            "atomic destination has no parent: {}",
            destination.display()
        )
    })?;
    let leaf = destination.file_name().ok_or_else(|| {
        eyre!(
            "atomic destination has no file name: {}",
            destination.display()
        )
    })?;
    Ok(parent.join(format!(
        ".{}.{}-tmp-{}-{id}",
        leaf.to_string_lossy(),
        label,
        std::process::id()
    )))
}

fn replace_from_ids(
    destination: &Path,
    body: &[u8],
    label: &str,
    ids: impl IntoIterator<Item = u64>,
) -> Result<()> {
    for id in ids {
        let temp = temp_path(destination, label, id)?;
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(e)
                    .wrap_err_with(|| format!("creating atomic temp file {}", temp.display()))
            }
        };

        if let Err(e) = file.write_all(body) {
            drop(file);
            let _ = fs::remove_file(&temp);
            return Err(e).wrap_err_with(|| format!("writing atomic temp file {}", temp.display()));
        }
        drop(file);

        if let Err(e) = fs::rename(&temp, destination) {
            let _ = fs::remove_file(&temp);
            return Err(e)
                .wrap_err_with(|| format!("atomically replacing {}", destination.display()));
        }
        return Ok(());
    }

    bail!(
        "could not allocate a unique atomic temp file for {}",
        destination.display()
    )
}

pub(crate) fn replace_atomically(destination: &Path, body: &[u8], label: &str) -> Result<()> {
    let ids = (0..128).map(|_| TEMP_ID.fetch_add(1, Ordering::Relaxed));
    replace_from_ids(destination, body, label, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn predicted_temp_links_are_never_followed() {
        let tmp = tempfile::tempdir().unwrap();

        let symlink_destination = tmp.path().join("state-symlink.json");
        let symlink_victim = tmp.path().join("symlink-victim.txt");
        fs::write(&symlink_victim, b"symlink victim").unwrap();
        let occupied_symlink_temp = temp_path(&symlink_destination, "test", 7).unwrap();
        std::os::unix::fs::symlink(&symlink_victim, &occupied_symlink_temp).unwrap();
        replace_from_ids(&symlink_destination, b"new state", "test", [7, 8]).unwrap();
        assert_eq!(fs::read(&symlink_victim).unwrap(), b"symlink victim");
        assert_eq!(fs::read(&symlink_destination).unwrap(), b"new state");
        assert!(fs::symlink_metadata(occupied_symlink_temp)
            .unwrap()
            .file_type()
            .is_symlink());

        let hardlink_destination = tmp.path().join("state-hardlink.json");
        let hardlink_victim = tmp.path().join("hardlink-victim.txt");
        fs::write(&hardlink_victim, b"hardlink victim").unwrap();
        let occupied_hardlink_temp = temp_path(&hardlink_destination, "test", 9).unwrap();
        fs::hard_link(&hardlink_victim, &occupied_hardlink_temp).unwrap();
        replace_from_ids(&hardlink_destination, b"newer state", "test", [9, 10]).unwrap();
        assert_eq!(fs::read(&hardlink_victim).unwrap(), b"hardlink victim");
        assert_eq!(fs::read(&hardlink_destination).unwrap(), b"newer state");
        assert_eq!(
            fs::read(occupied_hardlink_temp).unwrap(),
            b"hardlink victim"
        );
    }
}
