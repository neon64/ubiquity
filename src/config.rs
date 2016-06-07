use std::path::PathBuf;
use std::marker::PhantomData;
use generic_array::{GenericArray, ArrayLength};
use typenum::U2;

use regex::Regex;
use state::ArchiveEntryPerReplica;

/// A type representing an index into a list of replicas.
pub type ReplicaIndex = usize;

/// Internal trait which encapsulates the length of a `GenericArray<PathBuf>`
pub trait PathLen: ArrayLength<PathBuf> {}
impl<T: ArrayLength<PathBuf>> PathLen for T {}

/// Internal trait which encapsulates the length of a `GenericArray<ArchiveEntryPerReplica>`
pub trait ArchiveLen: ArrayLength<ArchiveEntryPerReplica> {}
impl<T: ArrayLength<ArchiveEntryPerReplica>> ArchiveLen for T {}

/// The configuration for the sync business.
#[derive(Debug)]
pub struct SyncInfo<PL: PathLen = U2, AL: ArchiveLen = U2> {
    pub roots: GenericArray<PathBuf, PL>,
    pub ignore: Ignore,
    pub compare_file_contents: bool,
    phantom_data: PhantomData<AL>
}

#[derive(Debug)]
/// Determines which files should be ignored when detecting updates.
pub struct Ignore {
    pub regexes: Vec<Regex>,
    pub paths: Vec<String>,
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

impl<PL: PathLen, AL: ArchiveLen> SyncInfo<PL, AL> {
    pub fn new(roots: GenericArray<PathBuf, PL>) -> Self {
        SyncInfo {
            roots: roots,
            ignore: Ignore::nothing(),
            compare_file_contents: true,
            phantom_data: PhantomData::<AL>
        }
    }
}
