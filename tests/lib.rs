extern crate env_logger;
extern crate ubiquity;
extern crate regex;
#[macro_use]
extern crate log;
#[macro_use]
extern crate generic_array;
extern crate typenum;

use typenum::U2;

use regex::Regex;

use std::path::{Path, PathBuf};
use std::fs;
use std::io;
use std::io::Write;

use ubiquity::config::*;
use ubiquity::detect;
use ubiquity::reconcile;
use ubiquity::propagate;
use ubiquity::archive::Archive;

fn set_up(name: &'static str) -> (Archive, SyncInfo) {
    let _ = env_logger::init();

    let archive_path = PathBuf::from(format!("tests/replicas/{}/archive", name));
    let a_path = PathBuf::from(format!("tests/replicas/{}/a", name));
    let b_path = PathBuf::from(format!("tests/replicas/{}/b", name));

    clean_directory(&archive_path).unwrap();
    clean_directory(&a_path).unwrap();
    clean_directory(&b_path).unwrap();

    let config = SyncInfo::new(arr![PathBuf; a_path, b_path]);

    let archive = Archive::new(archive_path).unwrap();

    return (archive, config)
}

#[test]
fn test_differences_are_empty() {
    let (archive, config) = set_up("differences_are_empty");

    let result = detect::find_updates(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::EmptyProgressCallback).unwrap();
    assert!(result.differences.is_empty());
}

#[test]
fn test_files_are_ignored() {
    let (archive, mut config) = set_up("files_are_ignored");
    config.ignore.regexes.push(Regex::new(r"foo").unwrap());
    config.ignore.paths.push("baz".to_owned());

    fs::File::create(config.roots[0].join("foo")).unwrap();
    fs::File::create(config.roots[0].join("something_contains_foo")).unwrap();
    fs::create_dir(config.roots[0].join("baz")).unwrap();

    let result = detect::find_updates(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::EmptyProgressCallback).unwrap();
    assert!(result.differences.is_empty());
}

#[test]
fn test_changes_are_detected() {
    let (archive, config) = set_up("changes_are_detected");

    let mut test_document = fs::File::create(config.roots[1].join("Test Document")).unwrap();
    write!(test_document, "Hello World").unwrap();

    let result = detect::find_updates(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::EmptyProgressCallback).unwrap();
    assert_eq!(result.differences.len(), 1);
    assert_eq!(&result.differences[0].path, Path::new("Test Document"));
}

#[test]
fn test_nested_differences_are_removed() {
    let (archive, config) = set_up("nested_differences_are_removed");

    fs::create_dir(config.roots[0].join("baz")).unwrap();
    fs::create_dir(config.roots[0].join("baz/qux")).unwrap();
    fs::File::create(config.roots[0].join("baz/qux/cub")).unwrap();

    let mut sd = detect::SearchDirectories::new(vec![Path::new("baz").into(), Path::new("baz/qux").into()], false);

    let result = detect::find_updates(&archive, &mut sd, &config, &detect::EmptyProgressCallback).unwrap();
    assert_eq!(result.differences.len(), 1);
    assert_eq!(&result.differences[0].path, Path::new("baz"));
}

#[test]
fn test_differences_are_resolved() {
    let (archive, config) = set_up("differences_are_resolved");
    let ref sd = detect::SearchDirectories::from_root();

    detect_and_resolve(&archive, &config, sd);

    // test creations
    fs::create_dir(config.roots[0].join("baz")).unwrap();
    fs::File::create(config.roots[0].join("baz/cub")).unwrap();

    info!("Testing after dir created");
    detect_and_resolve(&archive, &config, sd);

    info!("Checking replicas are in sync");
    let result = detect::find_updates(&archive, &mut sd.clone(), &config, &detect::EmptyProgressCallback).unwrap();
    assert_eq!(result.statistics.archive_additions, 0);
    assert_eq!(result.differences.len(), 0);

    // test deletions
    fs::remove_dir_all(config.roots[1].join("baz")).unwrap();

    info!("Testing after dir removed");
    detect_and_resolve(&archive, &config, sd);

    info!("Checking replicas are in sync");
    let result = detect::find_updates(&archive, &mut sd.clone(), &config, &detect::EmptyProgressCallback).unwrap();
    assert_eq!(result.differences.len(), 0);
    assert_eq!(result.statistics.archive_additions, 0);
}

#[test]
fn test_regex_forward_slash() {
    let r = regex::Regex::new(r"/target/").unwrap();
    assert!(r.is_match("/Users/bob/awesome/target/foo"));
    assert!(!r.is_match("/Users/bob/awesome/target"));
}

fn detect_and_resolve(archive: &Archive, config: &SyncInfo<U2>, search_directories: &detect::SearchDirectories) {
    let result = detect::find_updates(archive, &mut search_directories.clone(), config, &detect::EmptyProgressCallback).unwrap();

    info!("{} differences", result.differences.len());
    for difference in result.differences {
        let operation = reconcile::guess_operation(&difference);
        info!("difference {:?}: {:?}", difference.path, operation);
        if let reconcile::Operation::PropagateFromMaster(master) = operation {
            propagate::propagate(&difference, master, &archive, &propagate::DefaultPropagationOptions, &propagate::EmptyProgressCallback).unwrap();
        }
    }
}

fn clean_directory(p: &Path) -> io::Result<()> {
    if !p.exists() {
        fs::create_dir_all(p)?;
        return Ok(());
    }
    for entry in fs::read_dir(p)? {
        let entry = entry?;
        if entry.metadata()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
