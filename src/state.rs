use std::convert::From;
use std::path::PathBuf;
use regex::Regex;
use std::path::Path;
use std::os::unix::fs::DirEntryExt;
use std::os::unix::fs::MetadataExt;

pub type HashedPath = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub path_hash: HashedPath,
    //pub path: String,
    pub replicas: Vec<ArchiveEntryPerReplica>
}

impl ArchiveEntry {
    pub fn new(path_hash: HashedPath, replicas: Vec<ArchiveEntryPerReplica>) -> ArchiveEntry {
        ArchiveEntry {
            //path: path,
            path_hash: path_hash,
            replicas: replicas
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveEntryPerReplica {
    Empty,
    Directory(ArchiveEntryExists),
    File(ArchiveEntryExists),
    Symlink(ArchiveEntryExists)
}

impl ArchiveEntryPerReplica {
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

    pub fn is_file_or_symlink(&self) -> bool {
        match *self {
            ArchiveEntryPerReplica::File(_) | ArchiveEntryPerReplica::Symlink(_) => true,
            _ => false,
        }
    }

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Just like `ArchiveEntryPerReplica`, but it contains information about the current path on the filesystem.
pub struct CurrentEntryPerReplica {
    pub path: PathBuf,
    pub archive: ArchiveEntryPerReplica
}

impl CurrentEntryPerReplica {
    pub fn has_been_modified(&self) -> bool {
        self.archive != ArchiveEntryPerReplica::from(&*self.path)
    }
}

/// The configuration for the sync business.
#[derive(Debug)]
pub struct SyncInfo {
    pub roots: Vec<PathBuf>,
    pub ignore_regex: Vec<Regex>,
    pub ignore_path: Vec<String>,
    pub compare_file_contents: bool
}

impl SyncInfo {
    pub fn new() -> Self {
        SyncInfo {
            roots: Vec::new(),
            ignore_regex: Vec::new(),
            ignore_path: Vec::new(),
            compare_file_contents: true
        }
    }
}