use std::io;
use std::convert::From;
use std::hash::Hasher;
use std::fs::{File, create_dir, remove_file};
use std::path::{Path, PathBuf};
use bincode::serde::{serialize_into, deserialize_from, DeserializeError, SerializeError};
use bincode::SizeLimit;
use util::hash_single;
use byteorder::{Error as ByteorderError, WriteBytesExt, ReadBytesExt, LittleEndian};

use state::{HashedPath, ArchiveEntry, ArchiveEntryPerReplica};
use util::FnvHashMap;

const ARCHIVE_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
pub struct Archive {
    pub directory: PathBuf
}

impl Archive {
    pub fn new(directory: PathBuf) -> Result<Self, io::Error> {
        // creates the archive directory
        if !directory.exists() {
            try!(create_dir(&directory));
        }
        Ok(Archive { directory: directory })
    }

    fn file_for_directory(&self, directory: &Path) -> PathBuf {
        self.directory.join(hash_single(directory).to_string())
    }

    /// Returns an `ArchiveFile` struct which abstracts over operations on a single archive file.
    /// Remember each 'file' in the archive represents an entire directory (not recursive) in the replicas.
    pub fn for_directory(&self, directory: &Path) -> ArchiveFile {
        return ArchiveFile { path: self.file_for_directory(directory) }
    }
}

/// Abstracts over operations on a single archive file.
/// Remember each 'file' in the archive represents an entire directory (not recursive) in the replicas.
pub struct ArchiveFile {
    path: PathBuf
}

impl ArchiveFile {
    /// Remove all entries from this file.
    /// This just slightly more efficient than writing an empty Vec.
    pub fn remove_all(&self) -> Result<(), io::Error> {
        remove_file(self.path.clone())
    }

    /// Reads the archive entries into a Vec
    pub fn read(&self) -> Result<Vec<ArchiveEntry>, ReadError> {
        if self.path.exists() {
            let mut file = try!(File::open(self.path.clone()));
            match read_entries(&mut file) {
                Ok(i) => Ok(i),
                Err(ReadError::InvalidArchiveVersion(version)) => {
                    info!("Archive file {:?} using outdated version ({})", self.path, version);
                    Ok(Vec::new())
                },
                Err(e) => Err(From::from(e))
            }
        } else {
            Ok(Vec::new()) // an empty set of entries
        }
    }

    /// Writes a Vec of entries
    pub fn write(&self, mut entries: Vec<ArchiveEntry>) -> Result<(), WriteError> {
        remove_deleted_entries(&mut entries);

        if entries.is_empty() {
            if self.path.exists() {
                debug!("Removing archive file {:?} (all entries gone)", self.path);
                try!(remove_file(self.path.clone()));
            } else {
                debug!("Archive file {:?} doesn't exist (all entries gone)", self.path);
            }
        } else {
            debug!("Writing archive file {:?}", self.path);
            let ref mut file = try!(File::create(self.path.clone()));
            try!(write_entries(file, &entries));
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct ArchiveEntries {
    entries: FnvHashMap<HashedPath, Vec<ArchiveEntryPerReplica>>,
    pub dirty: bool
}

impl Into<ArchiveEntries> for Vec<ArchiveEntry> {
    fn into(self) -> ArchiveEntries {
        let mut a = ArchiveEntries::empty();
        for item in self {
            a.entries.insert(item.path_hash, item.replicas);
        };
        a
    }
}

impl ArchiveEntries {
    fn empty() -> Self {
        ArchiveEntries {
            entries: Default::default(),
            dirty: false
        }
    }

    pub fn to_vec(&self) -> Vec<ArchiveEntry> {
        let mut entries_vec = Vec::new();
        for (hash, info) in &self.entries {
            entries_vec.push(ArchiveEntry::new(*hash, info.clone()));
        }
        entries_vec
    }

    pub fn get(&self, path: &Path) -> Option<&Vec<ArchiveEntryPerReplica>> {
        self.entries.get(&hash_single(path))
    }

    pub fn insert(&mut self, path: &Path, entries: Vec<ArchiveEntryPerReplica>) {
        let hashed_path = hash_single(path);

        // warn because it means we are potentially being inefficient
        warn!("Inserting data into archive for path {:?} (hashed: {})\n", path, hashed_path);
        self.entries.insert(hashed_path, entries);
        self.dirty = true;
    }
}

// Loops through each ArchiveEntry and removes it if all replicas are empty.
// Without this archive sizes will probably explode.
fn remove_deleted_entries(entries: &mut Vec<ArchiveEntry>) {
    entries.retain(|entry| {
        let mut keep = false;
        for replica in &entry.replicas {
            match replica {
                &ArchiveEntryPerReplica::Empty => {},
                _ => { keep = true }
            }
        }

        if !keep {
            info!("Removing stale entry before writing to archive");
        }
        keep
    });
}

/// reads a set of entries from a binary stream
fn read_entries<R: io::Read>(read: &mut R) -> Result<Vec<ArchiveEntry>, ReadError> {
    let version = try!(read.read_u32::<LittleEndian>());
    if version != ARCHIVE_VERSION {
        return Err(ReadError::InvalidArchiveVersion(version))
    }
    let result = try!(deserialize_from(read, SizeLimit::Infinite));
    Ok(result)
}

// writes a set of entries to a binary stream
fn write_entries<W: io::Write>(out: &mut W, entries: &Vec<ArchiveEntry>) -> Result<(), WriteError> {
    try!(out.write_u32::<LittleEndian>(ARCHIVE_VERSION));
    try!(serialize_into(out, entries, SizeLimit::Infinite));
    Ok(())
}

#[derive(Debug)]
pub enum ReadError {
    InvalidArchiveVersion(u32),
    IoError(io::Error),
    ByteOrderError(ByteorderError),
    DeserializeError(DeserializeError)
}

impl From<DeserializeError> for ReadError {
    fn from(e: DeserializeError) -> Self {
        ReadError::DeserializeError(e)
    }
}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        ReadError::IoError(e)
    }
}

impl From<ByteorderError> for ReadError {
    fn from(e: ByteorderError) -> Self {
        ReadError::ByteOrderError(e)
    }
}

#[derive(Debug)]
pub enum WriteError {
    IoError(io::Error),
    ByteOrderError(ByteorderError),
    SerializeError(SerializeError)
}

impl From<SerializeError> for WriteError {
    fn from(e: SerializeError) -> Self {
        WriteError::SerializeError(e)
    }
}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        WriteError::IoError(e)
    }
}

impl From<ByteorderError> for WriteError {
    fn from(e: ByteorderError) -> Self {
        WriteError::ByteOrderError(e)
    }
}
