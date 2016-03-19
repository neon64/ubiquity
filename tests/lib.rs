#![feature(plugin)]

extern crate env_logger;
extern crate ubiquity;
extern crate regex;
#[macro_use]
extern crate log;

use regex::Regex;

use std::path::Path;
use std::fs;
use std::io;
use std::io::Write;

use ubiquity::state::SyncInfo;
use ubiquity::conflict::{detect, resolve};
use ubiquity::transfer;
use ubiquity::archive::Archive;

fn set_up() -> (Archive, SyncInfo) {
    let _ = env_logger::init();

    let archive = Archive::new(Path::new("tests/archive").to_path_buf()).unwrap();

    let config = SyncInfo {
        roots: vec![Path::new("tests/root_a").to_path_buf(), Path::new("tests/root_b").to_path_buf()],
        ignore_regex: vec![],
        ignore_path: vec![],
        compare_file_contents: true
    };

    clean_directory(Path::new("tests/archive")).unwrap();
    clean_directory(Path::new("tests/root_a")).unwrap();
    clean_directory(Path::new("tests/root_b")).unwrap();

    return (archive, config)
}

#[test]
fn test_conflicts_are_empty() {
    let (archive, config) = set_up();

    let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
    assert!(conflicts.is_empty());
}

#[test]
fn test_files_are_ignored() {
    let (archive, mut config) = set_up();
    config.ignore_regex.push(Regex::new(r"foo").unwrap());
    config.ignore_path.push("baz".to_owned());

    fs::File::create("tests/root_a/foo").unwrap();
    fs::File::create("tests/root_a/something_contains_foo").unwrap();
    fs::create_dir("tests/root_a/baz").unwrap();

    let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
    assert!(conflicts.is_empty());
}

#[test]
fn test_changes_are_detected() {
    let (archive, config) = set_up();

    let mut test_document = fs::File::create("tests/root_b/Test Document").unwrap();
    write!(test_document, "Hello World").unwrap();

    let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(&conflicts[0].path, Path::new("Test Document"));
}

#[test]
fn test_nested_conflicts_are_removed() {
    let (archive, config) = set_up();

    fs::create_dir("tests/root_a/baz").unwrap();
    fs::create_dir("tests/root_a/baz/qux").unwrap();
    fs::File::create("tests/root_a/baz/qux/cub").unwrap();

    let mut sd = detect::SearchDirectories::new(vec![Path::new("baz").into(), Path::new("baz/qux").into()], false);

    let (conflicts, _) = detect::find_conflicts(&archive, &mut sd, &config, &detect::NoProgress).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(&conflicts[0].path, Path::new("baz"));
}

#[test]
fn test_conflicts_are_resolved() {
    let (archive, config) = set_up();
    let ref sd = detect::SearchDirectories::from_root();

    detect_and_resolve(&archive, &config, sd);

    // test creations
    fs::create_dir("tests/root_a/baz").unwrap();
    fs::File::create("tests/root_a/baz/cub").unwrap();

    detect_and_resolve(&archive, &config, sd);

    let (conflicts, _) = detect::find_conflicts(&archive, &mut sd.clone(), &config, &detect::NoProgress).unwrap();
    assert_eq!(conflicts.len(), 0);

    // test deletions
    fs::remove_dir_all("tests/root_a/baz").unwrap();

    detect_and_resolve(&archive, &config, sd);

    let (conflicts, _) = detect::find_conflicts(&archive, &mut sd.clone(), &config, &detect::NoProgress).unwrap();
    assert_eq!(conflicts.len(), 0);
}

#[test]
fn test_regex_forward_slash() {
    let r = regex::Regex::new(r"/target/").unwrap();
    assert!(r.is_match("/Users/bob/awesome/target/foo"));
    assert!(!r.is_match("/Users/bob/awesome/target"));
}

fn detect_and_resolve(archive: &Archive, config: &SyncInfo, search_directories: &detect::SearchDirectories) {
    let (conflicts, _) = detect::find_conflicts(archive, &mut search_directories.clone(), config, &detect::NoProgress).unwrap();

    info!("{} conflicts", conflicts.len());
    for conflict in conflicts {
        let resolution = resolve::guess(&conflict);
        info!("Conflict {:?} (resolving using {:?})", conflict.path, resolution);
        if let Some(master) = resolution {
            transfer::resolve_conflict(&conflict, master, &archive, &Default::default()).unwrap();
        }
    }
}

fn clean_directory(p: &Path) -> io::Result<()> {
    if !p.exists() {
        try!(fs::create_dir(p));
        return Ok(());
    }
    for entry in try!(fs::read_dir(p)) {
        let entry = try!(entry);
        if try!(entry.metadata()).is_dir() {
            try!(fs::remove_dir_all(entry.path()));
        } else {
            try!(fs::remove_file(entry.path()));
        }
    }
    Ok(())
}