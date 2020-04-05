use std::convert::From;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use generic_array::GenericArray;
use serde::{Serialize, Deserialize};
use std::iter::FromIterator;

use NumRoots;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Mirrors the state of a path on the filesystem.
pub enum ArchiveEntryPerReplica {
    Empty,
    Directory(ArchiveEntryExists),
    File(ArchiveEntryExists),
    Symlink(ArchiveEntryExists)
}

/// TODO: This is potentialy dodgy, and has just been implemented to satisfy generic bounds for
/// deserializing a GenericArray.
/// Don't use this implementation manually.
impl Default for ArchiveEntryPerReplica {
    fn default() -> Self {
        ArchiveEntryPerReplica::Empty
    }
}

impl ArchiveEntryPerReplica {
    /// Creates an array of ArchiveEntryPerReplica instances that reflect the current state of `path` inside `roots`.
    pub fn from_roots<N: NumRoots>(roots: &[PathBuf], path: &Path) -> GenericArray<ArchiveEntryPerReplica, N> {
        GenericArray::from_iter(
            roots.iter().map(
                |root: &PathBuf| ArchiveEntryPerReplica::from(root.join(path).as_ref())
            )
        )
    }

    /// Returns true if the entries are equal in type but not necessarily in content.
    pub fn equal_ty(a: &ArchiveEntryPerReplica, b: &ArchiveEntryPerReplica) -> bool {
        match *a {
            ArchiveEntryPerReplica::Empty => match *b {
                ArchiveEntryPerReplica::Empty => true,
                _ => false
            },
            ArchiveEntryPerReplica::File(_) => match *b {
                ArchiveEntryPerReplica::File(_) => true,
                _ => false
            },
            ArchiveEntryPerReplica::Directory(_) => match *b {
                ArchiveEntryPerReplica::Directory(_) => true,
                _ => false
            },
            ArchiveEntryPerReplica::Symlink(_) => match *b {
                ArchiveEntryPerReplica::Symlink(_) => true,
                _ => false
            }
        }
    }

    /// Returns true if the entry is a file or a symlink
    pub fn is_file_or_symlink(&self) -> bool {
        match *self {
            ArchiveEntryPerReplica::File(_) | ArchiveEntryPerReplica::Symlink(_) => true,
            _ => false,
        }
    }

    /// Returns true if the entry is present (ie: it is not empty)
    pub fn entry_exists(&self) -> bool {
        match *self {
            ArchiveEntryPerReplica::Empty => false,
            _ => true,
        }
    }
}

impl<'a> From<&'a Path> for ArchiveEntryPerReplica {
    fn from(path: &'a Path) -> ArchiveEntryPerReplica {
        if !path.exists() {
            ArchiveEntryPerReplica::Empty
        } else {
            let metadata = path.metadata().unwrap();
            let entry = ArchiveEntryExists {
                ino: metadata.ino(),
                ctime: metadata.ctime()
            };
            let ty = metadata.file_type();
            if ty.is_file() {
                ArchiveEntryPerReplica::File(entry)
            } else if ty.is_dir() {
                ArchiveEntryPerReplica::Directory(entry)
            } else if ty.is_symlink() {
                ArchiveEntryPerReplica::Symlink(entry)
            } else {
                unreachable!()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntryExists {
    ino: u64,
    ctime: i64
}
