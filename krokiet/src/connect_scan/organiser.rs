use std::path::Path;
use std::rc::Rc;
use std::thread;

use czkawka_core::common::consts::DEFAULT_THREAD_SIZE;
use czkawka_core::common::tool_data::CommonData;
use czkawka_core::common::traits::Search;
use czkawka_core::common::{format_time, split_path};
use czkawka_core::tools::organiser;
use czkawka_core::tools::organiser::{FolderMergeGroup, Organiser};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::common::{MAX_INT_DATA_ORGANISER, MAX_STR_DATA_ORGANISER, split_u64_into_i32s};
use crate::connect_row_selection::checker::set_number_of_enabled_items;
use crate::connect_scan::{MessagesData, ScanData, get_dt_timestamp_string, get_text_messages, insert_data_to_model, reset_selection_at_end, set_common_settings};
use crate::connect_select::select_all;
use crate::{ActiveTab, GuiState, MainWindow, flk};

pub(crate) fn scan_organiser(a: Weak<MainWindow>, sd: ScanData) {
    thread::Builder::new()
        .stack_size(DEFAULT_THREAD_SIZE)
        .spawn(move || {
            let mut tool = Organiser::new();
            set_common_settings(&mut tool, &sd.custom_settings, &sd.stop_flag);
            tool.search(&sd.stop_flag, Some(&sd.progress_sender));

            let vector = tool.get_merge_groups().to_vec();
            let (critical, messages) = get_text_messages(&tool, &sd.basic_settings);

            let info = tool.get_information();
            let stopped_search = tool.get_stopped_search();
            sd.shared_models.lock().expect("Mutex poisoned").shared_organiser_state = Some(tool);

            let messages_data = MessagesData { critical, messages };

            a.upgrade_in_event_loop(move |app| {
                write_organiser_results(&app, vector, messages_data, info, sd, stopped_search);
            })
        })
        .expect("Cannot start thread - not much we can do here");
}

fn write_organiser_results(app: &MainWindow, vector: Vec<FolderMergeGroup>, messages_data: MessagesData, info: organiser::Info, sd: ScanData, stopped_search: bool) {
    let scanning_time_str = format_time(info.scanning_time);

    let items = Rc::new(VecModel::default());
    for group in vector {
        // Filled header row is the merge target, it is the row that gets checked and merged
        let (data_model_str, data_model_int) = prepare_data_model_organiser(&group.target_path, "", group.target_modified_date);
        insert_data_to_model(&items, data_model_str, data_model_int, Some(true));
        let (_, target_name) = split_path(&group.target_path);
        for entry in group.folders {
            let (data_model_str, data_model_int) = prepare_data_model_organiser(&entry.path, &target_name, Some(entry.modified_date));
            insert_data_to_model(&items, data_model_str, data_model_int, None);
        }
    }
    let (checked_items, _, model) = select_all(&items.into(), ActiveTab::Organiser);
    app.set_organiser_model(model);
    if let Some(critical) = messages_data.critical {
        app.invoke_scan_ended(critical.into());
    } else {
        if !stopped_search && sd.basic_settings.play_audio_on_scan_completion {
            sd.audio_player.play_scan_completed();
        }
        let result_message = flk!(
            "rust_found_folders_to_merge",
            items_found = info.number_of_folders_to_merge,
            groups = info.number_of_groups,
            time = scanning_time_str
        );
        if !stopped_search && sd.basic_settings.show_notification_on_scan_completion {
            crate::notification_manager::send_scan_completed_notification("Organiser", &result_message);
        }
        app.invoke_scan_ended(result_message.into());
    }
    app.global::<GuiState>().set_info_text(messages_data.messages.into());
    reset_selection_at_end(app, ActiveTab::Organiser);
    set_number_of_enabled_items(app, ActiveTab::Organiser, checked_items);
}

fn prepare_data_model_organiser(path: &Path, merge_into: &str, modified_date: Option<u64>) -> (ModelRc<SharedString>, ModelRc<i32>) {
    let (directory, folder_name) = split_path(path);
    // Target that does not exist yet has no modification date to show
    let modified_date_str = modified_date.map(get_dt_timestamp_string).unwrap_or_default();
    let data_model_str_arr: [SharedString; MAX_STR_DATA_ORGANISER] = [folder_name.into(), merge_into.into(), directory.into(), modified_date_str.into()];
    let data_model_str = VecModel::from_slice(&data_model_str_arr);
    let modification_split = split_u64_into_i32s(modified_date.unwrap_or_default());
    let data_model_int_arr: [i32; MAX_INT_DATA_ORGANISER] = [modification_split.0, modification_split.1];
    let data_model_int = VecModel::from_slice(&data_model_int_arr);
    (data_model_str, data_model_int)
}
