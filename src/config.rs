use std::path::PathBuf;
use typenum::U2;
use generic_array::GenericArray;

use regex::Regex;
use NumRoots;

/// The configuration for the sync business.
#[derive(Debug)]
pub struct SyncInfo<N: NumRoots = U2> {
    pub roots: GenericArray<PathBuf, N>,
    pub ignore: Ignore,
    pub compare_file_contents: bool
}

#[derive(Debug)]
/// Determines which files should be ignored when detecting updates.
pub struct Ignore {
    pub regexes: Vec<Regex>,
    pub paths: Vec<String>
}

impl Ignore {
    /// An `Ignore` struct that ignores nothing
    pub fn nothing() -> Self {
        Ignore {
            regexes: Vec::new(),
            paths: Vec::new()
        }
    }
}

impl<N: NumRoots> SyncInfo<N> {
    pub fn new(roots: GenericArray<PathBuf, N>) -> Self {
        SyncInfo {
            roots: roots,
            ignore: Ignore::nothing(),
            compare_file_contents: true
        }
    }
}
