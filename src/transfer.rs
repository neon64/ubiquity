use conflict::Conflict;
use std::fs;
use std::ffi::{OsStr};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;
use error::{SyncError, DescribeIoError};
use archive::{Archive, ArchiveEntries};
use state::{ArchiveEntryPerReplica};

/// Resolves a conflict, by making the replica indexed by 'master' the master copy.
/// All other replicas will be mirrored to match the master copy.
pub fn resolve_conflict(conflict: &Conflict, master: usize, archive: &Archive, options: &ConflictResolutionOptions) -> Result<(), SyncError> {
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
                ArchiveEntryPerReplica::Empty => try!(transfer_file(&master_entry.path, &replica.path, archive_update)),
                ArchiveEntryPerReplica::File(_) => try!(transfer_file(&master_entry.path, &replica.path, archive_update)),
                ArchiveEntryPerReplica::Directory(_) => {
                    try!(remove_directory_recursive(&replica.path, archive_update, options));
                    try!(transfer_file(&master_entry.path, &replica.path, archive_update));
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Directory(_) => match replica.archive {
                ArchiveEntryPerReplica::Empty => try!(transfer_directory(&master_entry.path, &replica.path, archive_update)),
                ArchiveEntryPerReplica::File(_) => {
                    try!(remove_file(&replica.path, archive_update, options));
                    try!(transfer_directory(&master_entry.path, &replica.path, archive_update));
                },
                ArchiveEntryPerReplica::Directory(_) => {
                    try!(remove_directory_recursive(&replica.path, archive_update, options));
                    try!(transfer_directory(&master_entry.path, &replica.path, archive_update));
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
        };
    }

    Ok(())
}

/// ConflictResolutionOptions allow the client to customize how files are transferred/deleted.
pub struct ConflictResolutionOptions<'a> {
    /// return false to cancel deleting a directory
    pub before_remove_dir: &'a Fn(&Path) -> bool,
    /// return `SyncError::Cancelled` to cancel deleting the file,
    /// otherwise delete the file/move it to the trash.
    pub remove_file: &'a Fn(&Path) -> Result<(), SyncError>,
    /// delete the directory and its contents
    pub remove_dir_all: &'a Fn(&Path) -> Result<(), SyncError>
}

static RETURN_TRUE: fn(&Path) -> bool = return_true;
static REMOVE_FILE: fn(&Path) -> Result<(), SyncError> = rm_file;
static REMOVE_DIR_ALL: fn(&Path) -> Result<(), SyncError> = rm_dir_all;

fn return_true(_: &Path) -> bool {
    true
}
fn rm_file(path: &Path) -> Result<(), SyncError> {
    try!(fs::remove_file(path));
    Ok(())
}
fn rm_dir_all(path: &Path) -> Result<(), SyncError> {
    try!(fs::remove_dir_all(path).describe(|| format!("when removing directory {:?}", path)));
    Ok(())
}

impl Default for ConflictResolutionOptions<'static> {
    fn default() -> ConflictResolutionOptions<'static> {
        ConflictResolutionOptions {
            before_remove_dir: &RETURN_TRUE as &'static Fn(&Path) -> bool,
            remove_file: &REMOVE_FILE as &'static Fn(&Path) -> Result<(), SyncError>,
            remove_dir_all: &REMOVE_DIR_ALL as &'static Fn(&Path) -> Result<(), SyncError>,
        }
    }
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

fn remove_file(path: &Path, archive_update: &ArchiveUpdateInfo, options: &ConflictResolutionOptions) -> Result<(), SyncError> {
    info!("Removing file {:?}", path);

    // delegate the actual removal to a callback function
    try!((options.remove_file)(path));
    /*if !(options.before_delete)(path) {
        return Err(SyncError::Cancelled);
    }
    try!(fs::remove_file(path));*/
    debug!("Updating archives");
    update_archive(archive_update)
}

fn remove_directory_recursive(path: &Path, archive_update: &ArchiveUpdateInfo, options: &ConflictResolutionOptions) -> Result<(), SyncError> {
    if !(options.before_remove_dir)(path) {
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
    try!((options.remove_dir_all)(path));
    //try!(fs::remove_dir_all(path).describe(|| format!("when removing directory {:?}", path)));
    update_archive(archive_update)
}

fn transfer_file(source: &Path, dest: &Path, archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    let parent = dest.parent().unwrap();
    if !parent.exists() {
        info!("Creating parent directory {:?}", parent);
        try!(fs::create_dir_all(parent));
    }
    info!("Transferring file {:?} to {:?}", source, dest);
    try!(run_rsync(source, dest));
    debug!("Updating archives");
    update_archive(archive_update)
}

fn transfer_directory(source: &Path, dest: &Path, archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    try!(fs::create_dir_all(dest));

    info!("Copying directory {:?}", dest);
    try!(run_rsync(source, dest).describe(|| format!("while copying directory from {:?} to {:?}", source, dest)));

    debug!("Updating archives");
    try!(update_archive(archive_update));
    update_archive_directory_contents(archive_update)
}

fn run_rsync(source: &Path, dest: &Path) -> io::Result<()> {
    let append_slash = try!(source.metadata()).is_dir();
    let mut source_str = source.to_string_lossy().into_owned();
    if append_slash {
        source_str.push_str("/");
    }
    let mut command = Command::new("/usr/local/bin/rsync");
    let command = command.arg("-a")
        .arg("--info=progress2")
        .arg(source_str)
        .arg(dest.to_string_lossy().as_ref());
    let status = try!(command.status());
    println!("{}", status);

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
