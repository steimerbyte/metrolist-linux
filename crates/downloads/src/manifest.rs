//! `manifest.json`: one file, atomically replaced, never trusted blindly.
//!
//! A corrupt manifest is moved aside to `manifest.json.bak` and the manager
//! starts empty with a warning (never a panic); file names read back from disk
//! are re-validated so a hand-edited manifest cannot make the manager delete
//! something outside its directory.

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

use ml_core::{MlError, MlResult};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::model::{Download, DownloadId, DownloadState};

pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub(crate) const MANIFEST_BAK: &str = "manifest.json.bak";
const MANIFEST_TMP: &str = "manifest.json.tmp";
pub(crate) const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    #[serde(default)]
    entries: Vec<Download>,
}

/// `true` for a single plain file name (no separators, not `.`/`..`, relative).
///
/// Postcondition: any name that passes can only ever address a direct child of
/// the download directory, so `join(remove_file)` cannot escape it.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && Path::new(name).is_relative()
        && Path::new(name).components().count() == 1
}

/// Read the manifest, tolerating every failure mode.
///
/// - missing → empty;
/// - unparseable → moved to [`MANIFEST_BAK`], empty, warning;
/// - `Downloading` entries → `Queued` (nothing is in flight at startup);
/// - completed entries with no file name → `Queued`;
/// - entries whose file name is not a plain name → dropped (never deleted).
pub(crate) fn load(dir: &Path) -> BTreeMap<DownloadId, Download> {
    let path = dir.join(MANIFEST_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            warn!(path = %path.display(), %error, "cannot read download manifest; starting empty");
            return BTreeMap::new();
        }
    };
    match serde_json::from_slice::<Manifest>(&bytes) {
        Ok(manifest) => normalize(manifest.entries),
        Err(error) => {
            let bak = dir.join(MANIFEST_BAK);
            let _ = fs::remove_file(&bak);
            match fs::rename(&path, &bak) {
                Ok(()) => warn!(
                    %error,
                    "corrupt download manifest moved to {}; starting empty",
                    bak.display()
                ),
                Err(rename) => warn!(
                    %error,
                    %rename,
                    path = %path.display(),
                    "corrupt download manifest could not be moved aside; starting empty"
                ),
            }
            BTreeMap::new()
        }
    }
}

fn normalize(entries: Vec<Download>) -> BTreeMap<DownloadId, Download> {
    let mut map = BTreeMap::new();
    for mut entry in entries {
        entry.id = entry.song.id.clone();
        if entry.id.is_empty() {
            warn!("dropping manifest entry without a song id");
            continue;
        }
        match entry.file_name.as_deref() {
            Some(name) if !is_plain_file_name(name) => {
                warn!(id = %entry.id, name, "dropping manifest entry with an unsafe file name");
                continue;
            }
            _ => {}
        }
        if entry.state == DownloadState::Downloading {
            // Nothing survives a restart mid-transfer; retry from byte 0.
            entry.state = DownloadState::Queued;
            entry.bytes_downloaded = 0;
        }
        if entry.state == DownloadState::Completed && entry.file_name.is_none() {
            entry.state = DownloadState::Queued;
        }
        map.insert(entry.id.clone(), entry);
    }
    map
}

/// Atomically replace the manifest: write `manifest.json.tmp`, fsync, rename.
pub(crate) fn save(dir: &Path, entries: &BTreeMap<DownloadId, Download>) -> MlResult<()> {
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        entries: entries.values().cloned().collect(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| MlError::Store(format!("manifest encode: {error}")))?;
    let tmp = dir.join(MANIFEST_TMP);
    let target = dir.join(MANIFEST_FILE);
    // shortcut: no fsync on the directory, so a power loss can lose the rename
    // (never corrupt it). Upgrade if the manifest becomes authoritative state.
    {
        let mut file = fs::File::create(&tmp).map_err(MlError::from)?;
        file.write_all(&bytes).map_err(MlError::from)?;
        file.sync_all().map_err(MlError::from)?;
    }
    fs::rename(&tmp, &target).map_err(MlError::from)
}

/// Delete every `dl_*.part` file in `dir` and return how many were removed.
///
/// Nothing can be in flight when a manager is constructed or torn down (the
/// manifest normalizes `Downloading` back to `Queued`), so every such file is
/// garbage left by a crash, an abort or a `SIGKILL`.
pub(crate) fn sweep_part_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with("dl_")
            && name.ends_with(".part")
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_core::{AudioQuality, Song};

    fn entry(id: &str, state: DownloadState) -> Download {
        let mut download = Download::new(
            Song {
                id: id.to_string(),
                title: "t".into(),
                ..Song::default()
            },
            AudioQuality::Auto,
            1,
        );
        download.state = state;
        download
    }

    #[test]
    fn missing_manifest_is_empty() {
        let dir = std::env::temp_dir().join(format!("ml-dl-manifest-{}", std::process::id()));
        assert!(load(&dir).is_empty());
    }

    #[test]
    fn unsafe_file_names_are_never_loaded() {
        assert!(is_plain_file_name("a - b [id].webm"));
        assert!(is_plain_file_name("dl_x.part"));
        assert!(!is_plain_file_name(""));
        assert!(!is_plain_file_name(".."));
        assert!(!is_plain_file_name("."));
        assert!(!is_plain_file_name("../etc/passwd"));
        assert!(!is_plain_file_name("/etc/passwd"));
        assert!(!is_plain_file_name("sub/file.webm"));
        assert!(!is_plain_file_name("a\0b"));

        let dir = std::env::temp_dir().join(format!("ml-dl-unsafe-{}", std::process::id()));
        let mut entries = BTreeMap::new();
        let mut bad = entry("bad", DownloadState::Completed);
        bad.file_name = Some("../escape.webm".into());
        entries.insert("bad".to_string(), bad);
        let mut good = entry("good", DownloadState::Completed);
        good.file_name = Some("ok.webm".into());
        entries.insert("good".to_string(), good);
        fs::create_dir_all(&dir).unwrap();
        save(&dir, &entries).unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("good"));
    }

    #[test]
    fn stale_part_files_are_swept_and_nothing_else_is_touched() {
        let dir = std::env::temp_dir().join(format!("ml-dl-sweep-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("dl_abc.part"), b"junk").unwrap();
        fs::write(dir.join("song [abc].webm"), b"audio").unwrap();
        fs::write(dir.join("other.part"), b"keep").unwrap();
        assert_eq!(sweep_part_files(&dir), 1);
        assert!(!dir.join("dl_abc.part").exists());
        assert!(dir.join("song [abc].webm").exists());
        assert!(dir.join("other.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inflight_state_is_normalized_on_load() {
        let dir = std::env::temp_dir().join(format!("ml-dl-norm-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("x".to_string(), entry("x", DownloadState::Downloading));
        save(&dir, &entries).unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded["x"].state, DownloadState::Queued);
        assert_eq!(loaded["x"].bytes_downloaded, 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
