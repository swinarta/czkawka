use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use log::{debug, info};

/// Why merging a folder into its target failed.
#[derive(Debug)]
pub enum MergeError {
    /// The source folder does not exist (e.g. it was already merged or moved).
    SourceMissing(PathBuf),
    /// The target path exists but is not a folder.
    TargetNotFolder(PathBuf),
    /// A filesystem operation failed on the given path.
    Io { path: PathBuf, error: io::Error },
    /// Some entries could not be moved because a different item with the same name already
    /// exists in the target. They are left in the source folder untouched.
    Conflicts { source: PathBuf, conflicting: Vec<PathBuf> },
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceMissing(path) => write!(f, "Folder {} no longer exists", path.display()),
            Self::TargetNotFolder(path) => write!(f, "Merge target {} exists but is not a folder", path.display()),
            Self::Io { path, error } => write!(f, "Failed to process {}: {error}", path.display()),
            Self::Conflicts { source, conflicting } => {
                let names = conflicting.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "Folder {} was only partly merged, items already existing in target were kept: {names}", source.display())
            }
        }
    }
}

impl std::error::Error for MergeError {}

/// Moves the whole content of `source` into `target` and removes `source` afterwards.
///
/// - If `target` does not exist, `source` is simply renamed to `target`.
/// - Sub-folders present in both are merged recursively.
/// - A file (or a file/folder mismatch) already present in `target` is never overwritten; it is kept
///   in `source`, which is then left in place, and reported as [`MergeError::Conflicts`].
///
/// Both paths should be on the same filesystem, since entries are moved with `fs::rename`.
pub fn merge_folder_into(source: &Path, target: &Path) -> Result<(), MergeError> {
    if !fs::symlink_metadata(source).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(MergeError::SourceMissing(source.to_path_buf()));
    }

    match fs::symlink_metadata(target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::rename(source, target).map_err(|error| MergeError::Io {
                path: source.to_path_buf(),
                error,
            })?;
            info!("Renamed {} to {}", source.display(), target.display());
            return Ok(());
        }
        Err(error) => {
            return Err(MergeError::Io {
                path: target.to_path_buf(),
                error,
            });
        }
        Ok(metadata) if !metadata.is_dir() => return Err(MergeError::TargetNotFolder(target.to_path_buf())),
        Ok(_) => {}
    }

    let mut conflicting = Vec::new();
    merge_folder_content(source, target, &mut conflicting)?;
    if !conflicting.is_empty() {
        return Err(MergeError::Conflicts {
            source: source.to_path_buf(),
            conflicting,
        });
    }

    fs::remove_dir(source).map_err(|error| MergeError::Io {
        path: source.to_path_buf(),
        error,
    })?;
    info!("Merged {} into {}", source.display(), target.display());
    Ok(())
}

fn merge_folder_content(source: &Path, target: &Path, conflicting: &mut Vec<PathBuf>) -> Result<(), MergeError> {
    let read_dir = fs::read_dir(source).map_err(|error| MergeError::Io {
        path: source.to_path_buf(),
        error,
    })?;
    for entry in read_dir {
        let entry = entry.map_err(|error| MergeError::Io {
            path: source.to_path_buf(),
            error,
        })?;
        let entry_path = entry.path();
        let destination = target.join(entry.file_name());

        let destination_metadata = match fs::symlink_metadata(&destination) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::rename(&entry_path, &destination).map_err(|error| MergeError::Io { path: entry_path.clone(), error })?;
                continue;
            }
            Err(error) => return Err(MergeError::Io { path: destination, error }),
        };

        let entry_is_dir = entry.file_type().map_err(|error| MergeError::Io { path: entry_path.clone(), error })?.is_dir();
        if entry_is_dir && destination_metadata.is_dir() {
            let conflicts_before = conflicting.len();
            merge_folder_content(&entry_path, &destination, conflicting)?;
            if conflicting.len() == conflicts_before {
                fs::remove_dir(&entry_path).map_err(|error| MergeError::Io { path: entry_path.clone(), error })?;
            }
        } else {
            debug!("Not merging {}, because {} already exists", entry_path.display(), destination.display());
            conflicting.push(entry_path);
        }
    }
    Ok(())
}
