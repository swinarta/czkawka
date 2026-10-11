use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crossbeam_channel::Sender;
use fun_time::fun_time;
use log::debug;
use rayon::prelude::*;

use crate::common::dir_traversal::{common_get_entry_data, common_get_metadata_dir, common_read_dir, get_modified_time};
use crate::common::directories::Directories;
use crate::common::items::ExcludedItems;
use crate::common::model::ToolType;
use crate::common::progress_data::{ProgressData, ToolStage};
use crate::common::progress_stop_handler::{check_if_stop_received, prepare_thread_handler_common};
use crate::common::tool_data::CommonToolData;
use crate::tools::organiser::{FolderMergeEntry, FolderMergeGroup, Info, Organiser};

#[derive(Debug)]
pub(crate) struct FoundFolder {
    pub(crate) path: PathBuf,
    pub(crate) parent: PathBuf,
    pub(crate) name: String,
    pub(crate) modified_date: u64,
}

impl Organiser {
    pub fn new() -> Self {
        Self {
            common_data: CommonToolData::new(ToolType::Organiser),
            information: Info::default(),
            merge_groups: Vec::new(),
        }
    }

    /// Groups of numbered folders to merge, sorted by parent and target; folders inside a group are sorted by number.
    pub fn get_merge_groups(&self) -> &[FolderMergeGroup] {
        &self.merge_groups
    }

    pub const fn get_information(&self) -> Info {
        self.information
    }

    #[fun_time(message = "collect_folders", level = "debug")]
    pub(crate) fn collect_folders(&mut self, stop_flag: &Arc<AtomicBool>, progress_sender: Option<&Sender<ProgressData>>) -> Option<Vec<FoundFolder>> {
        let progress_handler = prepare_thread_handler_common(progress_sender, ToolStage::CollectingFolders, 0, 0);
        let excluded_items = self.common_data.excluded_items.clone();
        let directories = self.common_data.directories.clone();
        let recursive_search = self.common_data.recursive_search;

        let mut folders_to_check: Vec<PathBuf> = directories.included_directories.clone();
        let mut found_folders = Vec::new();

        while !folders_to_check.is_empty() {
            if check_if_stop_received(stop_flag) {
                progress_handler.join_thread();
                return None;
            }

            let segments: Vec<(Vec<FoundFolder>, Vec<String>)> = folders_to_check
                .into_par_iter()
                .map(|current_folder| {
                    let mut warnings = Vec::new();
                    let subfolders = read_subfolders(&current_folder, &directories, &excluded_items, &mut warnings);
                    progress_handler.increase_items(subfolders.len());
                    (subfolders, warnings)
                })
                .collect();

            folders_to_check = Vec::new();
            for (subfolders, warnings) in segments {
                self.common_data.text_messages.warnings.extend(warnings);
                if recursive_search {
                    folders_to_check.extend(subfolders.iter().map(|folder| folder.path.clone()));
                }
                found_folders.extend(subfolders);
            }
        }

        progress_handler.join_thread();
        Some(found_folders)
    }

    pub(crate) fn find_folders_to_merge(&mut self, found_folders: Vec<FoundFolder>) {
        self.merge_groups = group_folders_to_merge(found_folders);
        self.information.number_of_groups = self.merge_groups.len();
        self.information.number_of_folders_to_merge = self.merge_groups.iter().map(|group| group.folders.len()).sum();
        debug!(
            "Found {} folders to merge into {} groups",
            self.information.number_of_folders_to_merge, self.information.number_of_groups
        );
    }
}

fn read_subfolders(current_folder: &Path, directories: &Directories, excluded_items: &ExcludedItems, warnings: &mut Vec<String>) -> Vec<FoundFolder> {
    let Some(read_dir) = common_read_dir(current_folder, warnings) else {
        return Vec::new();
    };

    let mut subfolders = Vec::new();
    for entry in &read_dir {
        let Some(entry_data) = common_get_entry_data(entry, warnings, current_folder) else {
            continue;
        };
        // DirEntry::file_type does not follow symlinks, so symlinked folders are skipped here
        if !entry_data.file_type().is_ok_and(|file_type| file_type.is_dir()) {
            continue;
        }

        let path = entry_data.path();
        if excluded_items.is_excluded(&path) || directories.is_excluded_dir(&path) {
            continue;
        }

        #[cfg(target_family = "unix")]
        if directories.exclude_other_filesystems() {
            match directories.is_on_other_filesystems(&path) {
                Ok(true) => continue,
                Err(e) => warnings.push(e),
                Ok(false) => (),
            }
        }

        let Some(metadata) = common_get_metadata_dir(entry_data, warnings, &path) else {
            continue;
        };
        let modified_date = get_modified_time(&metadata, warnings, &path, true);
        subfolders.push(FoundFolder {
            name: entry_data.file_name().to_string_lossy().to_string(),
            parent: current_folder.to_path_buf(),
            path,
            modified_date,
        });
    }
    subfolders
}

/// Groups sibling folders by their base name and returns every group that has at least two
/// members (counting an already existing plain-named folder).
pub(crate) fn group_folders_to_merge(found_folders: Vec<FoundFolder>) -> Vec<FolderMergeGroup> {
    let existing_folders: HashMap<(PathBuf, String), u64> = found_folders
        .iter()
        .map(|folder| ((folder.parent.clone(), folder.name.clone()), folder.modified_date))
        .collect();

    // BTreeMap keeps the output ordered by parent and base name
    let mut groups: BTreeMap<(PathBuf, String), Vec<FoundFolder>> = BTreeMap::new();
    for folder in found_folders {
        if let Some(base_name) = strip_number_suffix(&folder.name) {
            groups.entry((folder.parent.clone(), base_name.to_string())).or_default().push(folder);
        }
    }

    let mut merge_groups = Vec::new();
    for ((parent, base_name), mut numbered_folders) in groups {
        let target_modified_date = existing_folders.get(&(parent.clone(), base_name.clone())).copied();
        if numbered_folders.len() + usize::from(target_modified_date.is_some()) < 2 {
            continue;
        }
        numbered_folders.sort_by(|a, b| natord_like_cmp(&a.name, &b.name));
        merge_groups.push(FolderMergeGroup {
            target_path: parent.join(&base_name),
            target_modified_date,
            folders: numbered_folders
                .into_iter()
                .map(|folder| FolderMergeEntry {
                    path: folder.path,
                    modified_date: folder.modified_date,
                })
                .collect(),
        });
    }
    merge_groups
}

// Orders `A (2)` before `A (10)` by comparing the trailing number numerically
fn natord_like_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let number_of = |name: &str| -> Option<u64> {
        let digits: String = name.chars().rev().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
        digits.chars().rev().collect::<String>().parse().ok()
    };
    number_of(a).cmp(&number_of(b)).then_with(|| a.cmp(b))
}

/// Returns the base name of a folder name ending with a number suffix, or `None` if it has none.
///
/// Recognised patterns: `A (1)`, `A(1)` and `A 1` - all return `A`.
/// The base must be non-empty, so `(1)` or `12` alone return `None`.
pub fn strip_number_suffix(name: &str) -> Option<&str> {
    let base = if let Some(without_paren) = name.strip_suffix(')') {
        let (base, number) = without_paren.rsplit_once('(')?;
        is_number(number).then_some(base)?
    } else {
        let (base, number) = name.rsplit_once(' ')?;
        is_number(number).then_some(base)?
    };
    let base = base.trim_end();
    (!base.is_empty()).then_some(base)
}

fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}
