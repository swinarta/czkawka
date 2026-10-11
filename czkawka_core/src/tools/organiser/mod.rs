pub mod core;
pub mod merge;
#[cfg(test)]
mod tests;
pub mod traits;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::common::tool_data::CommonToolData;
use crate::common::traits::ResultEntry;

/// A numbered folder (e.g. `A (1)`, `A 2`) that can be merged into its sibling base-name folder (`A`).
#[derive(Clone, Debug, Serialize)]
pub struct FolderMergeEntry {
    /// Full path of the numbered folder, e.g. `/photos/A (1)`.
    pub path: PathBuf,
    pub modified_date: u64,
}

/// Numbered sibling folders sharing one base name, together with the folder they merge into.
#[derive(Clone, Debug, Serialize)]
pub struct FolderMergeGroup {
    /// Full path of the merge target, e.g. `/photos/A`. It may not exist yet.
    pub target_path: PathBuf,
    /// Modification date of the target, or `None` if the target did not exist when the scan ran.
    pub target_modified_date: Option<u64>,
    /// Numbered folders to merge into the target, ordered by their number.
    pub folders: Vec<FolderMergeEntry>,
}

impl ResultEntry for FolderMergeEntry {
    fn get_path(&self) -> &Path {
        &self.path
    }

    fn get_modified_date(&self) -> u64 {
        self.modified_date
    }

    fn get_size(&self) -> u64 {
        0
    }
}

/// Finds sibling folders whose names differ only by a trailing number (`A (1)`, `A (2)`, `A 3`)
/// and proposes merging them into one folder named after the shared base (`A`).
///
/// A group is reported only when it has at least two members, counting an existing plain `A`.
/// Only folders sharing the same parent are grouped. Use [`merge::merge_folder_into`] to apply a merge.
pub struct Organiser {
    common_data: CommonToolData,
    information: Info,
    merge_groups: Vec<FolderMergeGroup>,
}

#[derive(Default, Clone, Copy)]
pub struct Info {
    /// Number of merge groups, i.e. distinct merge targets found.
    pub number_of_groups: usize,
    /// Number of numbered folders that would be merged into a target.
    pub number_of_folders_to_merge: usize,
    pub scanning_time: Duration,
}

impl Default for Organiser {
    fn default() -> Self {
        Self::new()
    }
}
