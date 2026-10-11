use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tempfile::TempDir;

use crate::common::tool_data::CommonData;
use crate::common::traits::Search;
use crate::tools::organiser::Organiser;
use crate::tools::organiser::core::strip_number_suffix;
use crate::tools::organiser::merge::{MergeError, merge_folder_into};

fn scan(path: &Path, recursive: bool) -> Organiser {
    let mut organiser = Organiser::new();
    organiser.set_included_paths(vec![path.to_path_buf()]);
    organiser.set_recursive_search(recursive);
    organiser.search(&Arc::new(AtomicBool::new(false)), None);
    organiser
}

fn file_names(organiser: &Organiser) -> Vec<(String, String)> {
    organiser
        .get_merge_groups()
        .iter()
        .flat_map(|group| {
            let target_name = group.target_path.file_name().unwrap().to_string_lossy().to_string();
            group
                .folders
                .iter()
                .map(move |entry| (entry.path.file_name().unwrap().to_string_lossy().to_string(), target_name.clone()))
        })
        .collect()
}

#[test]
fn test_strip_number_suffix() {
    assert_eq!(strip_number_suffix("A (1)"), Some("A"));
    assert_eq!(strip_number_suffix("A(12)"), Some("A"));
    assert_eq!(strip_number_suffix("B 1"), Some("B"));
    assert_eq!(strip_number_suffix("My Photos 2"), Some("My Photos"));
    assert_eq!(strip_number_suffix("A"), None);
    assert_eq!(strip_number_suffix("A1"), None);
    assert_eq!(strip_number_suffix("A (x)"), None);
    assert_eq!(strip_number_suffix("A ()"), None);
    assert_eq!(strip_number_suffix("(1)"), None);
    assert_eq!(strip_number_suffix("12"), None);
    assert_eq!(strip_number_suffix(" 1"), None);
}

#[test]
fn test_groups_numbered_siblings() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();
    for name in ["A (1)", "A (2)", "B 1", "B 2", "C (1)", "Other"] {
        fs::create_dir(path.join(name)).unwrap();
    }

    let organiser = scan(path, true);

    assert_eq!(
        file_names(&organiser),
        vec![
            ("A (1)".to_string(), "A".to_string()),
            ("A (2)".to_string(), "A".to_string()),
            ("B 1".to_string(), "B".to_string()),
            ("B 2".to_string(), "B".to_string()),
        ]
    );
    assert_eq!(organiser.get_information().number_of_groups, 2);
    assert_eq!(organiser.get_information().number_of_folders_to_merge, 4);
}

#[test]
fn test_groups_hold_target_and_numbered_folders() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();
    for name in ["A (1)", "A (2)", "A (3)"] {
        fs::create_dir(path.join(name)).unwrap();
    }

    let organiser = scan(path, true);

    let groups = organiser.get_merge_groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].target_path, path.join("A"));
    assert_eq!(groups[0].target_modified_date, None);
    let folders: Vec<_> = groups[0].folders.iter().map(|entry| entry.path.clone()).collect();
    assert_eq!(folders, vec![path.join("A (1)"), path.join("A (2)"), path.join("A (3)")]);
}

#[test]
fn test_existing_plain_folder_counts_as_group_member() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();
    fs::create_dir(path.join("A")).unwrap();
    fs::create_dir(path.join("A (1)")).unwrap();

    let organiser = scan(path, true);

    assert_eq!(file_names(&organiser), vec![("A (1)".to_string(), "A".to_string())]);
    assert!(organiser.get_merge_groups()[0].target_modified_date.is_some());
}

#[test]
fn test_only_siblings_are_grouped_and_numbers_sorted() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();
    fs::create_dir_all(path.join("x").join("A (1)")).unwrap();
    fs::create_dir_all(path.join("y").join("A (2)")).unwrap();
    for name in ["D (10)", "D (2)"] {
        fs::create_dir(path.join(name)).unwrap();
    }

    let organiser = scan(path, true);

    assert_eq!(
        file_names(&organiser),
        vec![("D (2)".to_string(), "D".to_string()), ("D (10)".to_string(), "D".to_string())]
    );
}

#[test]
fn test_non_recursive_scans_only_first_level() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();
    fs::create_dir_all(path.join("sub").join("A (1)")).unwrap();
    fs::create_dir_all(path.join("sub").join("A (2)")).unwrap();

    assert!(scan(path, false).get_merge_groups().is_empty());
    assert_eq!(scan(path, true).get_information().number_of_folders_to_merge, 2);
}

#[test]
fn test_merge_renames_when_target_missing() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("A (1)");
    let target = temp_dir.path().join("A");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file.txt"), b"content").unwrap();

    merge_folder_into(&source, &target).unwrap();

    assert!(!source.exists());
    assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"content");
}

#[test]
fn test_merge_moves_content_and_merges_subfolders() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("A (2)");
    let target = temp_dir.path().join("A");
    fs::create_dir_all(source.join("sub")).unwrap();
    fs::create_dir_all(target.join("sub")).unwrap();
    fs::write(source.join("new.txt"), b"new").unwrap();
    fs::write(source.join("sub").join("inner.txt"), b"inner").unwrap();
    fs::write(target.join("sub").join("kept.txt"), b"kept").unwrap();

    merge_folder_into(&source, &target).unwrap();

    assert!(!source.exists());
    assert_eq!(fs::read(target.join("new.txt")).unwrap(), b"new");
    assert_eq!(fs::read(target.join("sub").join("inner.txt")).unwrap(), b"inner");
    assert_eq!(fs::read(target.join("sub").join("kept.txt")).unwrap(), b"kept");
}

#[test]
fn test_merge_never_overwrites_existing_files() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("A (1)");
    let target = temp_dir.path().join("A");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("same.txt"), b"from source").unwrap();
    fs::write(source.join("unique.txt"), b"unique").unwrap();
    fs::write(target.join("same.txt"), b"from target").unwrap();

    let result = merge_folder_into(&source, &target);

    let Err(MergeError::Conflicts { conflicting, .. }) = result else {
        panic!("Expected conflicts, got {result:?}");
    };
    assert_eq!(conflicting, vec![source.join("same.txt")]);
    assert_eq!(fs::read(target.join("same.txt")).unwrap(), b"from target");
    assert_eq!(fs::read(source.join("same.txt")).unwrap(), b"from source");
    assert_eq!(fs::read(target.join("unique.txt")).unwrap(), b"unique");
}

#[test]
fn test_merge_fails_when_target_is_file() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("A (1)");
    let target = temp_dir.path().join("A");
    fs::create_dir(&source).unwrap();
    fs::write(&target, b"file").unwrap();

    assert!(matches!(merge_folder_into(&source, &target), Err(MergeError::TargetNotFolder(_))));
    assert!(source.exists());
}

#[test]
fn test_merge_fails_when_source_missing() {
    let temp_dir = TempDir::new().unwrap();
    let source = temp_dir.path().join("A (1)");
    let target = temp_dir.path().join("A");

    assert!(matches!(merge_folder_into(&source, &target), Err(MergeError::SourceMissing(_))));
}
