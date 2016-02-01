use std::io;
use std::convert::From;
use std::hash::Hasher;
use std::fs::{File, create_dir, remove_file};
use std::path::{Path, PathBuf};
use bincode::serde::{serialize_into, deserialize_from, DeserializeError, SerializeError};
use bincode::SizeLimit;
use util::hash_single;
use byteorder::{Error as ByteorderError, WriteBytesExt, ReadBytesExt, LittleEndian};

use state::{ArchiveEntry, ArchiveEntryPerReplica};

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

    pub fn file_for_directory(&self, directory: &Path) -> PathBuf {
        self.directory.join(hash_single(directory).to_string())
    }

    pub fn get_entries_for_directory_or_empty(&self, directory: &Path) -> Result<Vec<ArchiveEntry>, ReadError> {
        let archive_file = self.file_for_directory(directory);
        if archive_file.exists() {
            let mut file = try!(File::open(archive_file.clone()));
            match read_entries(&mut file) {
                Ok(i) => Ok(i),
                Err(ReadError::InvalidVersion(version)) => {
                    info!("Archive file {:?} using outdated version ({})", archive_file, version);
                    Ok(Vec::new())
                },
                Err(e) => Err(From::from(e))
            }
        } else {
            Ok(Vec::new()) // an empty set of entries
        }
    }

    pub fn write_entries_for_directory(&self, directory: &Path, mut entries: Vec<ArchiveEntry>) -> Result<(), WriteError> {
        remove_deleted_entries(&mut entries);

        let archive_file = self.file_for_directory(directory);

        if entries.is_empty() {
            debug!("Removing archive file {:?} (all entries gone)", archive_file);
            try!(remove_file(archive_file));
            Ok(())
        } else {
            debug!("Writing archive file {:?}", archive_file);
            let ref mut file = try!(File::create(archive_file));
            write_entries(file, &entries)
        }
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

#[derive(Debug)]
pub enum ReadError {
    InvalidVersion(u32),
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

/// reads a set of entries from a binary stream
fn read_entries<R: io::Read>(read: &mut R) -> Result<Vec<ArchiveEntry>, ReadError> {
    let version = try!(read.read_u32::<LittleEndian>());
    if version != ARCHIVE_VERSION {
        return Err(ReadError::InvalidVersion(version))
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