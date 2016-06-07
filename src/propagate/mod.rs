use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use walkdir::WalkDir;

use detect::Difference;
use error::{SyncError, DescribeIoError};
use archive::{Archive, ArchiveEntries};
use state::{ArchiveEntryPerReplica};
use config::*;

mod progress;
pub use propagate::progress::{ProgressCallback, EmptyProgressCallback, ToCheck};

/// Propagates a change from `master` to every other replica.
pub fn propagate<T, P, PL, AL>(
        difference: &Difference<PL, AL>,
        master: usize,
        archive: &Archive,
        options: &T,
        progress: &P) -> Result<(), SyncError> where T: PropagationOptions, P: ProgressCallback, PL: PathLen, AL: ArchiveLen {
    let ref master_entry = difference.current_state[master];
    let master_path = difference.absolute_path_for_root(master);

    //let ref archive_update = ArchiveUpdateInfo::<PL, AL>::new(difference.path, &difference.roots, archive);

    for (i, replica) in difference.current_state.iter().enumerate() {
        // skip the master
        if i == master { continue; }

        let absolute_path = difference.absolute_path_for_root(i);
        if replica != &ArchiveEntryPerReplica::from(absolute_path.as_ref()) {
            return Err(SyncError::PathModified(absolute_path))
        }

        match *master_entry {
            ArchiveEntryPerReplica::Empty => match *replica {
                ArchiveEntryPerReplica::Empty => { },
                ArchiveEntryPerReplica::File(_) => remove_file(&absolute_path, options)?,
                ArchiveEntryPerReplica::Directory(_) => remove_directory_recursive(&absolute_path, options)?,
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::File(_) => match *replica {
                ArchiveEntryPerReplica::Empty => transfer_file(&master_path, &absolute_path, progress)?,
                ArchiveEntryPerReplica::File(_) => transfer_file(&master_path, &absolute_path, progress)?,
                ArchiveEntryPerReplica::Directory(_) => {
                    remove_directory_recursive(&absolute_path, options)?;
                    transfer_file(&master_path, &absolute_path, progress)?;
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Directory(_) => match *replica {
                ArchiveEntryPerReplica::Empty => transfer_directory(&master_path, &absolute_path, progress)?,
                ArchiveEntryPerReplica::File(_) => {
                    remove_file(&absolute_path, options)?;
                    transfer_directory(&master_path, &absolute_path, progress)?;
                },
                ArchiveEntryPerReplica::Directory(_) => {
                    remove_directory_recursive(&absolute_path, options)?;
                    transfer_directory(&master_path, &absolute_path, progress)?;
                },
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
            },
            ArchiveEntryPerReplica::Symlink(_) => unimplemented!()
        };
    }

    // Update the archives for this path and its children
    update_archive_for_path::<PL, AL>(&difference.path, archive, &difference.roots)?;

    Ok(())
}

/// Information required to update an entry in the archive.
/*struct ArchiveUpdateInfo<'a, PL: PathLen + 'a, AL: ArchiveLen> {
    archive: &'a Archive,
    roots: &'a GenericArray<PathBuf, PL>,
    path: Cow<'a, Path>,
    al: PhantomData<AL>
}

impl<'a, PL: PathLen, AL: ArchiveLen> ArchiveUpdateInfo<'a, PL, AL> {
    fn new<I>(path: I, roots: &'a GenericArray<PathBuf, PL>, archive: &'a Archive) -> Self where I: Into<Cow<'a, Path>> {
        ArchiveUpdateInfo {
            archive: archive,
            roots: roots,
            path: path.into(),
            al: PhantomData::<AL>
        }
    }

    fn for_child(&self, child: &OsStr) -> Self {
        ArchiveUpdateInfo::new(self.path.join(child.clone()), self.roots, self.archive)
    }
}*/

fn remove_file<T>(path: &Path, options: &T) -> Result<(), SyncError> where T: PropagationOptions {
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    info!("Removing file {:?}", path);
    // delegate the actual removal to a callback function
    options.remove_file(path)
}

fn remove_directory_recursive<T>(path: &Path, options: &T) -> Result<(), SyncError> where T: PropagationOptions {
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    info!("Removing directory {:?}", path);
    // delegate the actual removal to a callback function
    options.remove_dir_all(path)

    /*debug!("Removing archive files for contents of {:?}", path);
    for entry in WalkDir::new(path) {
        let entry = entry?;
        if entry.metadata()?.is_dir() {
            let child_path = archive_update.path.join(entry.path().strip_prefix(path).unwrap().as_os_str());
            let archive = archive_update.archive.for_directory(&child_path);
            archive.remove_all()?;
        }
    }*/

    //debug!("Removing archive file for directory {:?}", path);
    //archive_update.archive.for_directory(&archive_update.path).remove_all()?;


    //update_archive(archive_update)
}

fn transfer_file<P>(source: &Path, dest: &Path, progress: &P) -> Result<(), SyncError> where P: ProgressCallback {
    let parent = dest.parent().unwrap();
    if !parent.exists() {
        info!("Creating parent directory {:?}", parent);
        fs::create_dir_all(parent)?;
    }
    info!("Transferring file {:?} to {:?}", source, dest);
    run_rsync(source, dest, progress)
        .describe(|| format!("while copying file from {:?} to {:?}", source, dest))?;

    Ok(())
}

fn transfer_directory<P>(source: &Path, dest: &Path, progress: &P) -> Result<(), SyncError> where P: ProgressCallback {
    fs::create_dir_all(dest)?;

    info!("Copying directory {:?} to {:?}", source, dest);
    run_rsync(source, dest, progress)
        .describe(|| format!("while copying directory from {:?} to {:?}", source, dest))?;

    Ok(())
}

fn run_rsync<P>(source: &Path, dest: &Path, progress: &P) -> io::Result<()> where P: ProgressCallback {
    let append_slash = source.metadata()?.is_dir();
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
    let mut command = command.spawn()?;

    {
        let stdout = command.stdout.as_mut().unwrap();
        let reader = io::BufReader::new(stdout);

        progress::parse_from_stdout(reader, progress)?;
    }

    let status = command.wait()?;
    println!("{}", status);
    if !status.success() {
        panic!("Error in rsync");
    }

    Ok(())
}

/// Look at the archives in this path, and if it is a directory remove all descendants.
fn update_archive_for_path<PL: PathLen, AL: ArchiveLen>(relative_path: &Path, archive: &Archive, roots: &[PathBuf]) -> Result<(), SyncError> {
    let directory = relative_path.parent().unwrap();
    let mut archive_file = archive.for_directory(directory);
    let mut entries: ArchiveEntries<AL> = archive_file.read()?;

    // remove old archive information (only needed when `relative_path` is a directory)
    {
        let replicas = entries.get(relative_path);
        if let Some(replicas) = replicas {
            let is_dir = any_directories_in(&replicas);

            if is_dir {
                debug!("There are descendant directories inside {:?} that need to be cleared from the archive", relative_path);
                let mut stack = Vec::new();
                stack.push(Archive::hash(relative_path));
                loop {
                    let item = match stack.pop() { Some(v) => v, None => break };

                    trace!("Scanning archive file {:?} for descendant directories", item);
                    let mut archive_file = archive.for_hashed_directory(item);
                    let entries: ArchiveEntries<AL> = archive_file.read()?;

                    let dirs = entries.iter().filter(|&(_, replicas)| {
                        any_directories_in(&replicas)
                    }).map(|(hash, _)| *hash);
                    for dir in dirs {
                        stack.push(dir);
                    }

                    archive_file.remove_all()?;
                }
            } else {
                debug!("{:?} is not a directory, no pruning needed", relative_path);
            }
        } else {
            debug!("No entry {:?} in archive {}, no pruning needed", relative_path, archive_file);
        }
    }

    info!("Updating {:?} in {}", relative_path, archive_file);

    // update archives for this exact path
    let replicas = ArchiveEntryPerReplica::from_roots::<AL>(&roots, relative_path);
    entries.insert(relative_path, replicas);
    archive_file.write(&mut entries)?;

    // update archives for children of this path, only if it is a directory
    let first_root = roots[0].join(relative_path);
    if first_root.is_dir() {
        for entry in WalkDir::new(&first_root) {
            let entry = entry?;
            if entry.metadata()?.is_dir() {
                let dir_relative_path = relative_path.join(entry.path().strip_prefix(&first_root).unwrap().as_os_str());
                let mut entries = ArchiveEntries::<AL>::empty();

                for entry in entry.path().read_dir()? {
                    let entry = entry?;
                    if !entry.metadata()?.is_dir() {

                        let child_path = relative_path.join(entry.path().strip_prefix(&first_root).unwrap().as_os_str());
                        let replicas = ArchiveEntryPerReplica::from_roots::<AL>(&roots, &child_path);
                        entries.insert(&child_path, replicas)
                    }
                }

                let mut archive_file = archive.for_directory(&dir_relative_path);
                info!("Updating {}", archive_file);
                archive_file.write(&mut entries)?;
            }
        }
    }

    Ok(())
}

/// Searches to see if a directory exists at any of the replicas
fn any_directories_in(replicas: &[ArchiveEntryPerReplica]) -> bool {
    replicas.iter().any(|replica| {
        if let ArchiveEntryPerReplica::Directory(_) = *replica {
            true
        } else {
            false
        }
    })
}

/// PropagationOptions allow the client to customize how files are transferred/deleted.
pub trait PropagationOptions {
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

/// A zero-sized struct with a simple implementation of PropagationOptions.
pub struct DefaultPropagationOptions;

impl PropagationOptions for DefaultPropagationOptions {
    fn should_remove(&self, _: &Path) -> bool {
        true
    }
    fn remove_file(&self, path: &Path) -> Result<(), SyncError> {
        fs::remove_file(path)?;
        Ok(())
    }
    fn remove_dir_all(&self, path: &Path) -> Result<(), SyncError> {
        fs::remove_dir_all(path).describe(|| format!("when removing directory {:?}", path))?;
        Ok(())
    }
}