use crate::archive;
use std::fmt;
use std::io;
use std::path::PathBuf;
use walkdir::Error as WalkDirError;

#[derive(Debug)]
/// The many causes for an error during the synchronization process
pub enum SyncError {
    PathModified(PathBuf),
    IoError(io::Error, Option<String>),
    RootDoesntExist(PathBuf),
    AbsolutePathProvided(PathBuf),
    ArchiveReadError(archive::ReadError),
    ArchiveWriteError(archive::WriteError),
    /// The requested operation was cancelled before it could be completed.
    Cancelled,
    WalkDirError(WalkDirError),
    /// The rsync executable wasn't found
    RsyncNotFound(String),
}

impl From<io::Error> for SyncError {
    fn from(e: io::Error) -> Self {
        SyncError::IoError(e, None)
    }
}

impl From<(io::Error, String)> for SyncError {
    fn from(e: (io::Error, String)) -> Self {
        SyncError::IoError(e.0, Some(e.1))
    }
}

impl From<archive::ReadError> for SyncError {
    fn from(e: archive::ReadError) -> Self {
        SyncError::ArchiveReadError(e)
    }
}

impl From<archive::WriteError> for SyncError {
    fn from(e: archive::WriteError) -> Self {
        SyncError::ArchiveWriteError(e)
    }
}

impl From<WalkDirError> for SyncError {
    fn from(e: WalkDirError) -> Self {
        SyncError::WalkDirError(e)
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            SyncError::PathModified(ref path) => write!(f, "the file/directory at {:?} was modified by another application", path),
            SyncError::IoError(ref io, Some(ref message)) => write!(f, "io error: {}, ({})", io, message),
            SyncError::IoError(ref io, None) => write!(f, "io error: {}", io),
            SyncError::RootDoesntExist(ref root) => write!(f, "root does not exist: {:?}", root),
            SyncError::AbsolutePathProvided(ref path) => write!(f, "the absolute path {:?} is invalid (hint: search directories must be relative to the replica root)", path),
            SyncError::ArchiveWriteError(ref e) => write!(f, "archive write error: {:?}", e),
            SyncError::ArchiveReadError(ref e) => write!(f, "archive read error: {:?}", e),
            SyncError::Cancelled => write!(f, "operation cancelled"),
            SyncError::WalkDirError(ref e) => write!(f, "walk dir error: {:?}", e),
            SyncError::RsyncNotFound(ref path) => write!(f, "rsync executable not found at: {:?}", path)
        }
    }
}

pub trait DescribeIoError<T> {
    fn describe<F, I>(self, message: F) -> Result<T, (io::Error, String)>
    where
        F: Fn() -> I,
        I: Into<String>;
}

impl<T> DescribeIoError<T> for Result<T, io::Error> {
    fn describe<F, I>(self, message: F) -> Result<T, (io::Error, String)>
    where
        F: Fn() -> I,
        I: Into<String>,
    {
        self.map_err(|e| (e, message().into()))
    }
}
