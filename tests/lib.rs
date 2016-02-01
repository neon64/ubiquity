#![feature(plugin)]

extern crate env_logger;
extern crate ubiquity;
extern crate regex;

use regex::Regex;

use std::path::Path;
use std::fs;
use std::io;
use std::io::Write;
use std::collections::HashMap;

use ubiquity::state::SyncInfo;
use ubiquity::conflict::{detect, resolve};
use ubiquity::{transfer, archive};

#[test]
fn test_basic_sync() {
    env_logger::init().unwrap();

    let archive = archive::Archive::new(Path::new("tests/archive").to_path_buf()).unwrap();

    let config = SyncInfo {
        roots: vec![Path::new("tests/root_a").to_path_buf(), Path::new("tests/root_b").to_path_buf()],
        ignore_regex: vec![Regex::new(r"foo").unwrap()],
        ignore_path: vec!["baz".to_owned()],
        compare_file_contents: true
    };

    clean_directory(Path::new("tests/root_a")).unwrap();
    clean_directory(Path::new("tests/root_b")).unwrap();

    {
        let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
        assert!(conflicts.is_empty());
    }

    // test ignore
    {
        fs::File::create("tests/root_a/foo").unwrap();
        fs::File::create("tests/root_a/something_contains_foo").unwrap();
        fs::create_dir("tests/root_a/baz").unwrap();

        let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
        assert!(conflicts.is_empty());
    }

    // test basic detection
    {
        let mut test_document = fs::File::create("tests/root_b/Test Document").unwrap();
        write!(test_document, "Hello World").unwrap();

        let (conflicts, _) = detect::find_conflicts(&archive, &mut detect::SearchDirectories::from_root(), &config, &detect::NoProgress).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(&conflicts[0].path, Path::new("Test Document"));
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