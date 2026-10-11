use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
use czkawka_core::common::progress_data::{ProgressData, ToolStage};
use czkawka_core::helpers::delayed_sender::DelayedSender;
use czkawka_core::helpers::messages::{MessageLimit, Messages};
use czkawka_core::tools::organiser::merge::merge_folder_into;
use log::{debug, error};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::common::StrDataOrganiser;
use crate::connect_row_selection::checker::set_number_of_enabled_items;
use crate::connect_row_selection::reset_selection;
use crate::simpler_model::{SimplerSingleMainListModel, ToSlintModel};
use crate::{ActiveTab, Callabler, GuiState, MainWindow, flk};

struct MergeGroup {
    header: SimplerSingleMainListModel,
    members: Vec<SimplerSingleMainListModel>,
}

struct MergeOutcome {
    rows: Vec<SimplerSingleMainListModel>,
    errors: Vec<String>,
    merged: usize,
}

pub(crate) fn connect_merge(app: &MainWindow, progress_sender: Sender<ProgressData>, stop_flag: Arc<AtomicBool>) {
    let a = app.as_weak();
    app.global::<Callabler>().on_merge_folders(move || {
        let app = a.upgrade().expect("MainWindow dropped while callback is still live");
        let active_tab = app.global::<GuiState>().get_active_tab();
        assert_eq!(active_tab, ActiveTab::Organiser, "Merging folders is only available in Organiser tab");
        stop_flag.store(false, Ordering::Relaxed);

        let groups = group_rows(active_tab.get_tool_model(&app).iter().map(|row| SimplerSingleMainListModel::from(&row)));
        let folders_to_merge = count_folders_to_merge(&groups);
        if folders_to_merge == 0 {
            app.global::<GuiState>().set_info_text(flk!("rust_no_folders_merged").into());
            return;
        }

        app.set_processing(true);
        let weak_app = a.clone();
        let progress_sender = progress_sender.clone();
        let stop_flag = stop_flag.clone();
        thread::spawn(move || {
            let outcome = merge_checked_groups(groups, folders_to_merge, &progress_sender, &stop_flag);
            weak_app
                .upgrade_in_event_loop(move |app| apply_merge_outcome(&app, outcome, folders_to_merge, &stop_flag))
                .expect("Failed to update app after merging folders");
        });
    });
}

// Every group starts with a header row, the merge target, followed by its numbered member folders
fn group_rows(rows: impl Iterator<Item = SimplerSingleMainListModel>) -> Vec<MergeGroup> {
    let mut groups: Vec<MergeGroup> = Vec::new();
    for row in rows {
        if row.header_row {
            groups.push(MergeGroup { header: row, members: Vec::new() });
        } else {
            groups.last_mut().expect("Organiser model must start with a header row").members.push(row);
        }
    }
    groups
}

fn count_folders_to_merge(groups: &[MergeGroup]) -> usize {
    groups.iter().filter(|group| group.header.checked).map(|group| group.members.len()).sum()
}

fn merge_checked_groups(groups: Vec<MergeGroup>, folders_to_merge: usize, progress_sender: &Sender<ProgressData>, stop_flag: &AtomicBool) -> MergeOutcome {
    let mut progress = ProgressData::new(ToolStage::MovingFiles, 0, 0);
    progress.entries_to_check = folders_to_merge;
    send_progress(progress_sender, progress);
    let delayed_sender = DelayedSender::new(progress_sender.clone(), Duration::from_millis(100));

    let mut outcome = MergeOutcome {
        rows: Vec::new(),
        errors: Vec::new(),
        merged: 0,
    };
    for MergeGroup { mut header, members } in groups {
        let remaining_members = if header.checked && !stop_flag.load(Ordering::Relaxed) {
            header.checked = false;
            let target = row_path(&header);
            members
                .into_iter()
                .filter(|member| {
                    if stop_flag.load(Ordering::Relaxed) {
                        return true;
                    }
                    let merge_result = merge_folder_into(&row_path(member), &target);
                    progress.entries_checked += 1;
                    delayed_sender.send(progress);
                    match merge_result {
                        Ok(()) => {
                            outcome.merged += 1;
                            false
                        }
                        Err(e) => {
                            outcome.errors.push(e.to_string());
                            true
                        }
                    }
                })
                .collect()
        } else {
            members
        };

        // Group without members has nothing left to merge
        if !remaining_members.is_empty() {
            outcome.rows.push(header);
            outcome.rows.extend(remaining_members);
        }
    }

    send_progress(progress_sender, progress);
    debug!("Merged {} folders out of {folders_to_merge}, failed {}", outcome.merged, outcome.errors.len());
    outcome
}

fn row_path(row: &SimplerSingleMainListModel) -> PathBuf {
    Path::new(&row.val_str[StrDataOrganiser::Path as usize]).join(&row.val_str[StrDataOrganiser::Name as usize])
}

fn send_progress(progress_sender: &Sender<ProgressData>, progress: ProgressData) {
    if let Err(e) = progress_sender.send(progress) {
        error!("Failed to send progress data: {e}");
    }
}

fn apply_merge_outcome(app: &MainWindow, outcome: MergeOutcome, folders_to_merge: usize, stop_flag: &AtomicBool) {
    let active_tab = ActiveTab::Organiser;
    // Headers left checked are the groups skipped after stop was requested
    let checked_items = outcome.rows.iter().filter(|row| row.checked).count();
    let mut new_model = outcome.rows.to_vec_model();
    for row in &mut new_model {
        row.focused_row = false;
    }
    active_tab.set_tool_model(app, ModelRc::new(VecModel::from(new_model)));

    let errors_number = outcome.errors.len();
    app.global::<GuiState>()
        .set_info_text(Messages::new_from_errors(outcome.errors).create_messages_text(MessageLimit::NoLimit).into());
    app.global::<GuiState>().set_preview_visible(false);
    reset_selection(app, active_tab, true);
    set_number_of_enabled_items(app, active_tab, checked_items as u64);
    stop_flag.store(false, Ordering::Relaxed);
    app.invoke_processing_ended(flk!("rust_merge_summary", merged = outcome.merged, failed = errors_number, total = folders_to_merge).into());
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossbeam_channel::unbounded;
    use tempfile::TempDir;

    use super::*;

    fn row(name: &str, path: &Path, header_row: bool, checked: bool) -> SimplerSingleMainListModel {
        SimplerSingleMainListModel {
            checked,
            filled_header_row: header_row,
            header_row,
            focused_row: false,
            val_int: vec![0, 0],
            val_str: vec![name.to_string(), String::new(), path.to_string_lossy().to_string(), String::new()],
        }
    }

    fn names(rows: &[SimplerSingleMainListModel]) -> Vec<String> {
        rows.iter().map(|row| row.val_str[StrDataOrganiser::Name as usize].clone()).collect()
    }

    #[test]
    fn merges_only_checked_groups_and_drops_finished_ones() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();
        for name in ["A (1)", "A (2)", "B 1", "B 2"] {
            fs::create_dir(path.join(name)).unwrap();
            fs::write(path.join(name).join(format!("{name}.txt")), b"x").unwrap();
        }
        let rows = vec![
            row("A", path, true, true),
            row("A (1)", path, false, false),
            row("A (2)", path, false, false),
            row("B", path, true, false),
            row("B 1", path, false, false),
            row("B 2", path, false, false),
        ];
        let groups = group_rows(rows.into_iter());
        assert_eq!(count_folders_to_merge(&groups), 2);

        let (sender, _receiver) = unbounded();
        let outcome = merge_checked_groups(groups, 2, &sender, &AtomicBool::new(false));

        assert_eq!(outcome.merged, 2);
        assert!(outcome.errors.is_empty());
        assert_eq!(names(&outcome.rows), vec!["B", "B 1", "B 2"]);
        assert!(path.join("A").join("A (1).txt").exists());
        assert!(path.join("A").join("A (2).txt").exists());
        assert!(!path.join("A (1)").exists());
        assert!(path.join("B 1").exists());
    }

    #[test]
    fn failed_members_stay_under_unchecked_header() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();
        fs::create_dir(path.join("A (1)")).unwrap();
        let rows = vec![row("A", path, true, true), row("A (1)", path, false, false), row("A (2)", path, false, false)];

        let (sender, _receiver) = unbounded();
        let outcome = merge_checked_groups(group_rows(rows.into_iter()), 2, &sender, &AtomicBool::new(false));

        assert_eq!(outcome.merged, 1);
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(names(&outcome.rows), vec!["A", "A (2)"]);
        assert!(!outcome.rows[0].checked);
    }
}
