//! The syncing process consists of three stages.
//! First you call `ubiquity::detect::find_updates` with a few of arguments
//! which tell Ubiquity where to look for changed files. It will return a list of files that differ between replicas.
//!
//! You can do whatever you like with that list, but most often you want to resolve those differences.
//! You can use an algorithm, user input, or a hardcoded value to determine which replica is the 'master' replica for each difference.
//! The 'master' is the correct, most up to date version of the file that will be propagated to all other replicas.
//!
//! Ubiquity comes with the `ubiquity::reconcile::guess_operation` function which will pick whichever file changed since the last run, or `None` if no files change, or if files changed on both sides.
//!
//! Once you have the 'master' replica, you can propagate changes using `ubiquity::propagate::propagate`.
//!
//! # Examples
//! ```
//! extern crate ubiquity;
//! extern crate regex;
//! extern crate typenum;
//! #[macro_use]
//! extern crate generic_array;
//!
//! use std::path::{Path, PathBuf};
//! use std::fs;
//! use ubiquity::{archive, detect, reconcile, propagate};
//! use ubiquity::config::{SyncInfo, Ignore};
//! use regex::Regex;
//!
//! fn main() {
//!     let archive = archive::Archive::new(Path::new("tests/replicas/archives").to_path_buf()).unwrap();
//!
//!     fs::create_dir(Path::new("tests/replicas/path_a")).unwrap();
//!     fs::create_dir(Path::new("tests/replicas/path_b")).unwrap();
//!
//!     let mut config: SyncInfo = SyncInfo::new(arr![PathBuf; PathBuf::from("tests/replicas/path_a"), PathBuf::from("tests/replicas/path_b")]);
//!     config.ignore.regexes.push(Regex::new(r".DS_Store").unwrap());
//!     config.ignore.paths.push("Microsoft User Data".to_string());
//!
//!     let mut search = detect::SearchDirectories::from_root();
//!
//!     let result = detect::find_updates(&archive, &mut search, &config, &detect::EmptyProgressCallback).expect("Failed to find conflicts");
//!
//!     if result.differences.is_empty() {
//!         println!("All in sync");
//!     }
//!
//!     for difference in result.differences {
//!         let mut operation = reconcile::guess_operation(&difference);
//!         println!("Difference at {:?}, resolving using {:?}", difference.path, operation);
//!         if let reconcile::Operation::PropagateFromMaster(master) = operation {
//!             propagate::propagate(&difference, master, &archive, &propagate::DefaultPropagationOptions, &propagate::EmptyProgressCallback).unwrap();
//!         }
//!     }
//! }
//! ```

#![feature(plugin)]
#![feature(question_mark)]
#![feature(custom_derive)]
#![plugin(serde_macros)]

extern crate fnv;
#[macro_use]
extern crate bincode;
extern crate serde;
extern crate regex;
extern crate byteorder;
#[macro_use]
extern crate log;
extern crate walkdir;
extern crate fs2;
extern crate generic_array;
extern crate typenum;

/// Detects differences between replicas
pub mod detect;
/// Makes suggestions on how to resolve differences between replicas
pub mod reconcile;
/// Propagates changes from a master replica to all others
pub mod propagate;

/// Handles the serialization and deserialization of archive data
pub mod archive;
/// Core structures for representing the state of the filesystem
pub mod state;
/// Configuration for the whole system
pub mod config;
/// Error handling
pub mod error;

mod compare_files;
mod util;