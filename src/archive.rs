use std::io;
use std::io::Seek;
use std::convert::From;
use std::fs::{OpenOptions, File, create_dir_all, remove_file};
use std::path::{Path, PathBuf};
use bincode::serde::{serialize_into, deserialize_from, DeserializeError, SerializeError};
use bincode::SizeLimit;
use util::hash_single;
use serde;
use byteorder::{Error as ByteorderError, WriteBytesExt, ReadBytesExt, LittleEndian};
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
    path: PathBuf,
    file: Option<File>
}

impl ArchiveFile {

    /// Creates a new wrapper around the given archive file.
    pub fn new(path: PathBuf) -> ArchiveFile {
        ArchiveFile { path: path, file: None }
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
    /// This may acquire (or wait for) a lock,
    /// ensuring that multiple threads/processes aren't reading/writing to/from the same archive file.
    pub fn read(&mut self) -> Result<Vec<ArchiveEntry>, ReadError> {
        if let Some(ref mut file) = self.file {
            read_from_file(file, &self.path)
        } else if self.path.exists()  {
            let mut file = try!(self.open_file());
            let res = read_from_file(&mut file, &self.path);
            self.file = Some(file);
            res
        } else {
            Ok(Vec::new()) // an empty set of entries
        }
    }

    fn open_file(&self) -> Result<File, io::Error> {
        let file = try!(OpenOptions::new().read(true).write(true).create(true).open(&self.path));
        trace!("Acquiring shared lock for {:?}", self.path);
        file.lock_exclusive().unwrap();
        trace!("Acquired lock");
        Ok(file)
    }

    /// Writes a Vec of entries
    pub fn write<I>(&mut self, entries: I) -> Result<(), WriteError> where I: Into<Vec<ArchiveEntry>> {
        let entries = &mut entries.into();
        remove_deleted_entries(entries);
        if entries.is_empty() {
            if self.path.exists() {
                debug!("Removing archive file {:?} (because entries are empty)", self.path);
                self.file = None;
                try!(remove_file(self.path.clone()));
            } else {
                debug!("Tried to remove archive file {:?} (because entries are empty), but it doesn't exist ", self.path);
            }
        } else if let Some(ref mut file) = self.file {
            try!(write_to_file(file, &self.path, &entries));
        } else {
            let mut file = try!(self.open_file());
            try!(write_to_file(&mut file, &self.path, &entries));
            self.file = Some(file);
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

impl From<Vec<ArchiveEntry>> for ArchiveEntries {
    fn from(vec: Vec<ArchiveEntry>) -> ArchiveEntries {
        let mut a = ArchiveEntries::empty();
        for item in vec {
            a.entries.insert(item.path_hash, item.replicas);
        };
        a
    }
}
impl Into<Vec<ArchiveEntry>> for ArchiveEntries {
    fn into(self) -> Vec<ArchiveEntry> {
        self.to_vec()
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
        trace!("{:#?}", entries);
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

fn read_from_file(file: &mut File, path: &Path) -> Result<Vec<ArchiveEntry>, ReadError> {
    debug!("Reading archive file {:?}", path);
    try!(file.seek(io::SeekFrom::Start(0)));
    match read_entries(file) {
        Ok(i) => Ok(i),
        Err(ReadError::InvalidArchiveVersion(version)) => {
            error!("Invalid archive version {} for file {:?}", version, path);
            Ok(Vec::new())
        },
        Err(ReadError::DeserializeError(DeserializeError::Serde(serde::de::value::Error::EndOfStream))) | Err(ReadError::ByteOrderError(ByteorderError::UnexpectedEOF)) => {
            error!("End of stream when reading archive file at path {:?}", path);
            Ok(Vec::new())
        },
        Err(e) => Err(From::from(e))
    }
}

fn write_to_file(file: &mut File, path: &Path, entries: &Vec<ArchiveEntry>) -> Result<(), WriteError> {
    info!("Writing archive file {:?}", path);
    try!(file.set_len(0));

    let pos = try!(file.seek(io::SeekFrom::Start(0)));
    assert_eq!(pos, 0);

    try!(write_entries(file, entries));

    Ok(())
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
