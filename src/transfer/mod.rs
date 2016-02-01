use conflict::Conflict;
use std::fs;
use std::ffi::{OsStr};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;
use util::hash_single;
use error::SyncError;
use archive::Archive;
use state::{ArchiveEntry, ArchiveEntryPerReplica};

pub struct ConflictResolutionOptions<'a> {
    pub before_delete: &'a Fn(&Path) -> bool
}

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
        /*if master_exists {
            try!(copy_between(&master_entry.path, &replica.path));
        } else {
            try!(remove_at_path(&replica.path));
        }*/
    }

    Ok(())
}

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
    if !(options.before_delete)(path) {
        return Err(SyncError::Cancelled);
    }
    try!(fs::remove_file(path));
    update_archive(archive_update)
}

fn remove_directory_recursive(path: &Path, archive_update: &ArchiveUpdateInfo, options: &ConflictResolutionOptions) -> Result<(), SyncError> {
    info!("Removing directory {:?}", path);
    if !(options.before_delete)(path) {
        return Err(SyncError::Cancelled);
    }

    for entry in try!(fs::read_dir(path)) {
        let entry = try!(entry);

        let ref child_archive_update = archive_update.for_child(&entry.file_name());

        if try!(entry.metadata()).is_dir() {
            try!(remove_directory_recursive(&entry.path(), child_archive_update, options));
        } else {
            try!(remove_file(&entry.path(), child_archive_update, options));
        }
    }
    try!(fs::remove_dir(path));
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
    update_archive(archive_update)
}

fn transfer_directory(source: &Path, dest: &Path, archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    try!(fs::create_dir_all(dest));

    info!("Copying directory {:?}", dest);
    try!(run_rsync(source, dest));

    info!("Updating archives");
    for entry in WalkDir::new(source) {
        let entry = try!(entry);
        println!("{}", entry.path().display());

        let ref child_archive_update = archive_update.for_child(entry.path().strip_prefix(dest).unwrap().as_os_str());
        try!(update_archive(child_archive_update));
    }

    /*for entry in try!(fs::read_dir(source)) {
        let entry = try!(entry);



        if try!(entry.metadata()).is_dir() {
            try!(transfer_directory(&entry.path(), &dest.join(entry.file_name()), child_archive_update));
        } else {
            try!(transfer_file(&entry.path(), &dest.join(entry.file_name()), child_archive_update));
        }
    }*/

    Ok(())
}

fn run_rsync(source: &Path, dest: &Path) -> io::Result<()> {
    let append_slash = try!(source.metadata()).is_dir();
    let mut source_str = source.to_string_lossy().into_owned();
    if append_slash {
        source_str.push_str("/");
    }
    let mut command = Command::new("rsync");
    let command = command.arg("-av")
        .arg(source_str)
        .arg(dest.to_string_lossy().as_ref());
    let status = try!(command.status());
    println!("{}", status);

    Ok(())
}

fn update_archive(archive_update: &ArchiveUpdateInfo) -> Result<(), SyncError> {
    let directory = archive_update.relative_path.parent().unwrap();
    let mut entries = try!(archive_update.archive.get_entries_for_directory_or_empty(directory));
    let path_hash = hash_single(&archive_update.relative_path);
    let mut found = false;
    for entry in &mut entries {
        if entry.path_hash == path_hash {
            trace!("Updating data in archive for path {:?} (hashed: {})", archive_update.relative_path, path_hash);
            entry.replicas = entries_for_paths(archive_update.absolute_paths.iter().map(|path| path.as_path()));
            found = true;
        }
    }
    // insert if it wasn't found
    if !found {
        trace!("Inserting data into archive for path {:?} (hashed: {})", archive_update.relative_path, path_hash);
        let replicas = entries_for_paths(archive_update.absolute_paths.iter().map(|path| path.as_path()));
        entries.push(ArchiveEntry::new(path_hash, replicas));
    }

    try!(archive_update.archive.write_entries_for_directory(directory, entries));
    Ok(())
}

// takes a list of paths and spits out new instances of `ArchiveEntryPerReplica`
fn entries_for_paths<'a, I>(current_state: I) -> Vec<ArchiveEntryPerReplica> where I: Iterator<Item=&'a Path> {
    current_state.map(|path| ArchiveEntryPerReplica::from(path)).collect()
}