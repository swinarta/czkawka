use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crossbeam_channel::Sender;
use fun_time::fun_time;

use crate::common::model::WorkContinueStatus;
use crate::common::progress_data::ProgressData;
use crate::common::tool_data::{CommonData, CommonToolData};
use crate::common::traits::{AllTraits, DebugPrint, DeletingItems, PrintResults, Search};
use crate::tools::organiser::{Info, Organiser};

impl AllTraits for Organiser {}

impl Search for Organiser {
    #[fun_time(message = "find_folders_to_organise", level = "info")]
    fn search(&mut self, stop_flag: &Arc<AtomicBool>, progress_sender: Option<&Sender<ProgressData>>) {
        let start_time = Instant::now();

        let () = (|| {
            if self.prepare_items(None).is_err() {
                return;
            }
            let Some(found_folders) = self.collect_folders(stop_flag, progress_sender) else {
                self.common_data.stopped_search = true;
                return;
            };
            self.find_folders_to_merge(found_folders);
        })();

        self.information.scanning_time = start_time.elapsed();

        if !self.common_data.stopped_search {
            self.debug_print();
        }
    }
}

impl DebugPrint for Organiser {
    #[expect(clippy::print_stdout)]
    fn debug_print(&self) {
        if !cfg!(debug_assertions) || cfg!(test) {
            return;
        }

        println!("---------------DEBUG PRINT---------------");
        println!("Number of groups - {}", self.information.number_of_groups);
        println!("Number of folders to merge - {}", self.information.number_of_folders_to_merge);
        self.debug_print_common();
        println!("-----------------------------------------");
    }
}

impl PrintResults for Organiser {
    fn write_results<T: Write>(&self, writer: &mut T) -> std::io::Result<()> {
        self.write_base_search_paths(writer)?;
        if self.merge_groups.is_empty() {
            return write!(writer, "Not found any folders to merge.");
        }

        writeln!(writer, "--------------------------Folders to merge--------------------------")?;
        writeln!(
            writer,
            "Found {} folders to merge into {} folders",
            self.information.number_of_folders_to_merge, self.information.number_of_groups
        )?;
        for group in &self.merge_groups {
            writeln!(writer, "{}", group.target_path.display())?;
            for entry in &group.folders {
                writeln!(writer, "    {}", entry.path.display())?;
            }
        }
        Ok(())
    }

    fn save_results_to_file_as_json(&self, file_name: &str, pretty_print: bool) -> std::io::Result<()> {
        self.save_results_to_file_as_json_internal(file_name, &self.merge_groups, pretty_print)
    }
}

impl CommonData for Organiser {
    type Info = Info;
    type Parameters = ();

    fn get_information(&self) -> Self::Info {
        self.information
    }
    fn get_params(&self) -> Self::Parameters {}
    fn get_cd(&self) -> &CommonToolData {
        &self.common_data
    }
    fn get_cd_mut(&mut self) -> &mut CommonToolData {
        &mut self.common_data
    }
    fn found_any_items(&self) -> bool {
        self.information.number_of_folders_to_merge > 0
    }
}

impl DeletingItems for Organiser {
    // Folders are merged via `merge::merge_folder_into`, never deleted by the scan itself
    fn delete_files(&mut self, _stop_flag: &Arc<AtomicBool>, _progress_sender: Option<&Sender<ProgressData>>) -> WorkContinueStatus {
        WorkContinueStatus::Continue
    }
}
