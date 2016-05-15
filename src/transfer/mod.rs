use conflict::Conflict;
use std::fs;
use std::ffi::{OsStr};
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use walkdir::WalkDir;
use error::{SyncError, DescribeIoError};
use archive::{Archive, ArchiveEntries};
use state::{ArchiveEntryPerReplica};

pub mod progress;

/// Resolves a conflict, by making the replica indexed by 'master' the master copy.
/// All other replicas will be mirrored to match the master copy.
pub fn resolve_conflict<T: TransferOptions, P: progress::Callback>(conflict: &Conflict, master: usize, archive: &Archive, options: &T, progress: &P) -> Result<(), SyncError> {
    let ref master_entry = conflict.current_state[master];

    let ref archive_update = ArchiveUpdateInfo {
        archive: archive,
        relative_path: conflict.path.to_path_buf(),
        absolute_paths: conflict.current_state.iter().map(|current| current.path.clone()).collect()
    };

    for (i, replica) in conflict.current_state.iter().enumerate() {
        // skip the master
        if i == master { continue; }

        if replica.has_been_modified() {
            return Err(SyncError::PathModified(replica.path.clone()))
        }

        match master_entry.archive {
            ArchiveEntryPerReplica::Empty => match replica.archive {
                ArchiveEntryPerReplica::Empty => { },
                ArchiveEntryPerReplica::File(_) => try!(remove_file(&replica.path, archive_update, options)),
                ArchiveEntryPerReplica::Directory(_) => try!(remove_directory_recursive(&replica.path, archive_update, options)), // delete DIRECTORY (updating archives)
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::File(_) => match replica.archive {
                ArchiveEntryPerReplica::Empty => try!(transfer_file(&master_entry.path, &replica.path, archive_update, progress)),
                ArchiveEntryPerReplica::File(_) => try!(transfer_file(&master_entry.path, &replica.path, archive_update, progress)),
                ArchiveEntryPerReplica::Directory(_) => {
                    try!(remove_directory_recursive(&replica.path, archive_update, options));
                    try!(transfer_file(&master_entry.path, &replica.path, archive_update, progress));
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Directory(_) => match replica.archive {
                ArchiveEntryPerReplica::Empty => try!(transfer_directory(&master_entry.path, &replica.path, archive_update, progress)),
                ArchiveEntryPerReplica::File(_) => {
                    try!(remove_file(&replica.path, archive_update, options));
                    try!(transfer_directory(&master_entry.path, &replica.path, archive_update, progress));
                },
                ArchiveEntryPerReplica::Directory(_) => {
                    try!(remove_directory_recursive(&replica.path, archive_update, options));
                    try!(transfer_directory(&master_entry.path, &replica.path, archive_update, progress));
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
        };
    }

    Ok(())
}

/// Information required to update an entry in the archive.
struct ArchiveUpdateInfo<'a> {
    archive: &'a Archive,
    relative_path: PathBuf,
    // read new replica metadata from these paths
    absolute_paths: Vec<PathBuf>
}

impl<'a> ArchiveUpdateInfo<'a> {
    fn for_child(&self, child: &OsStr) -> Self {
        ArchiveUpdateInfo {
            archive: self.archive,
            relative_path: self.relative_path.join(child.clone()),
            absolute_paths: self.absolute_paths.iter().map(|path| path.join(child.clone())).collect()
        }
    }
}

fn remove_file<T: TransferOptions>(path: &Path, archive_update: &ArchiveUpdateInfo, options: &T) -> Result<(), SyncError> {
    info!("Removing file {:?}", path);

    // delegate the actual removal to a callback function
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    try!(options.remove_file(path));
    debug!("Updating archives");
    update_archive(archive_update)
}

fn remove_directory_recursive<T: TransferOptions>(path: &Path, archive_update: &ArchiveUpdateInfo, options: &T) -> Result<(), SyncError> {
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    debug!("Removing archive files for contents of {:?}", path);
    for entry in WalkDir::new(path) {
        let entry = try!(entry);
        if try!(entry.metadata()).is_dir() {
            let child_path = archive_update.relative_path.join(entry.path().strip_prefix(path).unwrap().as_os_str());
            let archive = archive_update.archive.for_directory(&child_path);
            try!(archive.remove_all());
        }
    }

    debug!("Removing archive file for directory {:?}", path);
    try!(archive_update.archive.for_directory(&archive_update.relative_path).remove_all());

    info!("Removing directory {:?}", path);
    try!(options.remove_dir_all(path));
    update_archive(archive_update)
}

fn transfer_file<P: progress::Callback>(source: &Path, dest: &Path, archive_update: &ArchiveUpdateInfo, progress: &P) -> Result<(), SyncError> {
    let parent = dest.parent().unwrap();
    if !parent.exists() {
        info!("Creating parent directory {:?}", parent);
        try!(fs::create_dir_all(parent));
    }
    info!("Transferring file {:?} to {:?}", source, dest);
    try!(run_rsync(source, dest, progress));
    debug!("Updating archives");
    update_archive(archive_update)
}

fn transfer_directory<P: progress::Callback>(source: &Path, dest: &Path, archive_update: &ArchiveUpdateInfo, progress: &P) -> Result<(), SyncError> {
    try!(fs::create_dir_all(dest));

    info!("Copying directory {:?}", dest);
    try!(run_rsync(source, dest, progress).describe(|| format!("while copying directory from {:?} to {:?}", source, dest)));

    debug!("Updating archives");
    try!(update_archive(archive_update));
    update_archive_directory_contents(archive_update)
}

fn run_rsync<P: progress::Callback>(source: &Path, dest: &Path, progress: &P) -> io::Result<()> {
    let append_slash = try!(source.metadata()).is_dir();
    let mut source_str = source.to_string_lossy().into_owned();
    if append_slash {
        source_str.push_str("/");
    }
    let mut command = process::Command::new("/usr/local/bin/rsync");
    let command = command.arg("-a")
        .arg("--info=progress2")
        .arg(source_str)
        .stdout(process::Stdio::piped())
        .arg(dest.to_string_lossy().as_ref());
    let mut command = try!(command.spawn());

    {
        let stdout = command.stdout.as_mut().unwrap();
        let reader = io::BufReader::new(stdout);

        try!(progress::parse_from_stdout(reader, progress));
    }

    let status = try!(command.wait());
    println!("{}", status);
    if !status.success() {
        panic!("Error in rsync");
    }

    Ok(())
}

fn update_archive_directory_contents(archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    let mut archive = archive_update.archive.for_directory(&archive_update.relative_path);
    let mut entries: ArchiveEntries = try!(archive.read()).into();

    let ref dir = archive_update.absolute_paths[0];
    for entry in try!(fs::read_dir(dir.clone())) {
        let entry = try!(entry);

        let child_archive_update = archive_update.for_child(&entry.file_name());
        let replicas = entries_for_paths(child_archive_update.absolute_paths.iter().map(|path| path.as_path()));
        entries.insert(&child_archive_update.relative_path, replicas);

        if try!(entry.metadata()).is_dir() {
            try!(update_archive_directory_contents(&child_archive_update));
        }
    }

    try!(archive.write(entries));

    Ok(())
}

fn update_archive(archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    let directory = archive_update.relative_path.parent().unwrap();
    let mut archive = archive_update.archive.for_directory(directory);
    let mut entries: ArchiveEntries = try!(archive.read()).into();

    let replicas = entries_for_paths(archive_update.absolute_paths.iter().map(|path| path.as_path()));
    entries.insert(&archive_update.relative_path, replicas);

    try!(archive.write(entries));

    Ok(())
}

// takes a list of paths and spits out new instances of `ArchiveEntryPerReplica`
fn entries_for_paths<'a, I>(current_state: I) -> Vec<ArchiveEntryPerReplica> where I: Iterator<Item=&'a Path> {
    current_state.map(|path| ArchiveEntryPerReplica::from(path)).collect()
}


/// ConflictResolutionOptions allow the client to customize how files are transferred/deleted.
pub trait TransferOptions {
    /// return false to cancel deleting a directory
    fn should_remove(&self, &Path) -> bool;

    /// return `SyncError::Cancelled` to cancel deleting the file,
    /// otherwise delete the file/move it to the trash.
    /// This must return an error if the file was not removed successfully.
    fn remove_file(&self, &Path) -> Result<(), SyncError>;

    /// Delete the directory and its contents
    /// This must return an error if the directory was not removed successfully.
    /// Ignoring errors will mean that Ubiquity writes to the archive files when
    /// the replicas are still out of sync, resulting in an inconsistent state.
    fn remove_dir_all(&self, &Path) -> Result<(), SyncError>;

}

pub struct DefaultTransferOptions;

impl TransferOptions for DefaultTransferOptions {
    fn should_remove(&self, _: &Path) -> bool {
        true
    }
    fn remove_file(&self, path: &Path) -> Result<(), SyncError> {
        try!(fs::remove_file(path));
        Ok(())
    }
    fn remove_dir_all(&self, path: &Path) -> Result<(), SyncError> {
        try!(fs::remove_dir_all(path).describe(|| format!("when removing directory {:?}", path)));
        Ok(())
    }
}