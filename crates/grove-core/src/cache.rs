//! The last Snapshot on disk, so windows open instantly and refresh in place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::{SettingsFile, write_atomic};
use crate::snapshot::Snapshot;

/// Bump when `Snapshot`'s shape changes; other versions are ignored, not migrated.
const VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SnapshotCache {
    path: PathBuf,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

#[derive(Serialize, Deserialize)]
struct CacheFile<S> {
    version: u32,
    snapshot: S,
}

impl SnapshotCache {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `snapshot.json` next to the settings file.
    pub fn beside(settings: &SettingsFile) -> Self {
        Self::new(settings.path().with_file_name("snapshot.json"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The cached snapshot of `code_dir`. Missing, unreadable, corrupt, other-version and
    /// other-folder caches are all just a miss.
    pub fn load(&self, code_dir: &Path) -> Option<Snapshot> {
        let bytes = fs::read(&self.path).ok()?;
        let header: Header = serde_json::from_slice(&bytes).ok()?;
        if header.version != VERSION {
            return None;
        }
        let file: CacheFile<Snapshot> = serde_json::from_slice(&bytes).ok()?;
        Some(file.snapshot).filter(|s| s.code_dir == code_dir)
    }

    pub fn save(&self, snapshot: &Snapshot) -> io::Result<()> {
        let file = CacheFile {
            version: VERSION,
            snapshot,
        };
        let json = serde_json::to_vec(&file).map_err(io::Error::other)?;
        write_atomic(&self.path, &json)
    }
}
