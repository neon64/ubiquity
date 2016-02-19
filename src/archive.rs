use std::io;
use std::convert::From;
use std::hash::Hasher;
use std::fs::{File, create_dir_all, remove_file};
use std::path::{Path, PathBuf};
use bincode::serde::{serialize_into, deserialize_from, DeserializeError, SerializeError};
use bincode::SizeLimit;
use util::hash_single;
use byteorder::{Error as ByteorderError, WriteBytesExt, ReadBytesExt, LittleEndian};
//use file_lock::{Error, Lock, LockKind, AccessMode};
use fs2::FileExt;

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
            try!(create_dir_all(&directory));
        }
        Ok(Archive { directory: directory })
    }

    /// Returns an `ArchiveFile` struct which abstracts over operations on a single archive file.
    /// Remember each 'file' in the archive represents an entire directory (not recursive) in the replicas.
    pub fn for_directory(&self, directory: &Path) -> ArchiveFile {
        let path = self.directory.join(hash_single(directory).to_string());

        ArchiveFile::new(path)
    }
}

/// Abstracts over operations on a single archive file.
/// Remember each 'file' in the archive represents an entire directory (not recursive) in the replicas.
pub struct ArchiveFile {
    path: PathBuf
}

impl ArchiveFile {

    /// Creates a new wrapper around the given archive file.
    /// Through doing so this will acquire (or wait for) a lock,
    /// ensuring that multiple threads/processes aren't writing to the same archive file.
    pub fn new(path: PathBuf) -> ArchiveFile {
        /*let lock_path = path.with_extension("lock");
        let lock_file = OpenOptions::new().create(true).open(lock_path).unwrap();
        let lock = Lock::new(lock_file.as_raw_fd());
        match lock.lock(LockKind::NonBlocking, AccessMode::Write) {
            Ok(_) => (),
            Err(Error::Errno(i))
              => println!("Got filesystem error while locking {}", i),
        }*/

        if path.exists() {
            let f = File::open(path.clone()).unwrap();
            trace!("Acquiring lock for {:?}", path);
            f.lock_exclusive().unwrap();
            trace!("Acquired lock");
        }

        ArchiveFile { path: path }
    }

    /// Remove all entries from this file.
    /// This just slightly more efficient than writing an empty Vec.
    pub fn remove_all(&self) -> Result<(), io::Error> {
        if self.path.exists() {
            remove_file(self.path.clone())
        } else {
            Ok(())
        }
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
                debug!("Removing archive file {:?} (because entries are empty)", self.path);
                try!(remove_file(self.path.clone()));
            } else {
                debug!("Tried to remove archive file {:?} (because entries are empty), but it doesn't exist ", self.path);
            }
        } else {
            debug!("Writing archive file {:?}", self.path);
            let ref mut file = try!(File::create(self.path.clone()));
            try!(write_entries(file, &entries));
        }

        Ok(())
    }
}

impl Drop for ArchiveFile {
    fn drop(&mut self) {
        if self.path.exists() {
            let f = File::open(self.path.clone()).unwrap();
            trace!("Unlocking archive file {:?}", self.path);
            f.unlock().unwrap();
            trace!("Unlocked");
        }
        //self.lock.unlock().unwrap();
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

        // this means we are potentially being inefficient
        info!("Inserting data into archive for path {:?} (hashed: {})", path, hashed_path);
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
            info!("Removing empty entry before writing to archive");
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
