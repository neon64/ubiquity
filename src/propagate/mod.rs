use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;
use walkdir::WalkDir;

use crate::archive::{Archive, ArchiveEntries};
use crate::detect::Difference;
use crate::error::{DescribeIoError, SyncError};
use crate::state::ArchiveEntryPerReplica;
use crate::NumRoots;

mod progress;
pub use crate::propagate::progress::{EmptyProgressCallback, ProgressCallback, ToCheck};

/// Propagates a change from `master` to every other replica.
pub fn propagate<T, P, N>(
    difference: &Difference<N>,
    master: usize,
    archive: &Archive,
    options: &T,
    progress: &P,
) -> Result<(), SyncError>
where
    T: PropagationOptions,
    P: ProgressCallback,
    N: NumRoots,
{
    let master_entry = &difference.current_state[master];
    let master_path = difference.absolute_path_for_root(master);

    for (i, replica) in difference.current_state.iter().enumerate() {
        // skip the master
        if i == master {
            continue;
        }

        let absolute_path = difference.absolute_path_for_root(i);
        if replica != &ArchiveEntryPerReplica::from(absolute_path.as_ref()) {
            return Err(SyncError::PathModified(absolute_path));
        }

        match *master_entry {
            ArchiveEntryPerReplica::Empty => match *replica {
                ArchiveEntryPerReplica::Empty => {}
                ArchiveEntryPerReplica::File(_) => remove_file(&absolute_path, options)?,
                ArchiveEntryPerReplica::Directory(_) => {
                    remove_directory_recursive(&absolute_path, options)?
                }
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!(),
            },
            ArchiveEntryPerReplica::File(_) => match *replica {
                ArchiveEntryPerReplica::Empty => {
                    transfer_file(&master_path, &absolute_path, progress)?
                }
                ArchiveEntryPerReplica::File(_) => {
                    transfer_file(&master_path, &absolute_path, progress)?
                }
                ArchiveEntryPerReplica::Directory(_) => {
                    remove_directory_recursive(&absolute_path, options)?;
                    transfer_file(&master_path, &absolute_path, progress)?;
                }
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!(),
            },
            ArchiveEntryPerReplica::Directory(_) => match *replica {
                ArchiveEntryPerReplica::Empty => {
                    transfer_directory(&master_path, &absolute_path, progress)?
                }
                ArchiveEntryPerReplica::File(_) => {
                    remove_file(&absolute_path, options)?;
                    transfer_directory(&master_path, &absolute_path, progress)?;
                }
                ArchiveEntryPerReplica::Directory(_) => {
                    remove_directory_recursive(&absolute_path, options)?;
                    transfer_directory(&master_path, &absolute_path, progress)?;
                }
                ArchiveEntryPerReplica::Symlink(_) => unimplemented!(),
            },
            ArchiveEntryPerReplica::Symlink(_) => unimplemented!(),
        };
    }

    // Update the archives for this path and its children
    update_archive_for_path::<N>(&difference.path, archive, &difference.roots)?;

    Ok(())
}

fn remove_file<T>(path: &Path, options: &T) -> Result<(), SyncError>
where
    T: PropagationOptions,
{
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    info!("Removing file {:?}", path);
    // delegate the actual removal to a callback function
    options.remove_file(path)
}

fn remove_directory_recursive<T>(path: &Path, options: &T) -> Result<(), SyncError>
where
    T: PropagationOptions,
{
    if !options.should_remove(path) {
        return Err(SyncError::Cancelled);
    }

    info!("Removing directory {:?}", path);
    // delegate the actual removal to a callback function
    options.remove_dir_all(path)
}

fn transfer_file<P>(source: &Path, dest: &Path, progress: &P) -> Result<(), SyncError>
where
    P: ProgressCallback,
{
    let parent = dest.parent().unwrap();
    if !parent.exists() {
        info!("Creating parent directory {:?}", parent);
        fs::create_dir_all(parent)?;
    }
    info!("Transferring file {:?} to {:?}", source, dest);
    run_rsync(source, dest, progress)
    //.describe(|| format!("while copying file from {:?} to {:?}", source, dest))?;
}

fn transfer_directory<P>(source: &Path, dest: &Path, progress: &P) -> Result<(), SyncError>
where
    P: ProgressCallback,
{
    fs::create_dir_all(dest)?;

    info!("Copying directory {:?} to {:?}", source, dest);
    run_rsync(source, dest, progress)
    //.describe(|| format!("while copying directory from {:?} to {:?}", source, dest))?;
}

fn run_rsync<P>(source: &Path, dest: &Path, progress: &P) -> Result<(), SyncError>
where
    P: ProgressCallback,
{
    let rsync = "rsync";
    let append_slash = source.metadata()?.is_dir();
    let mut source_str = source.to_string_lossy().into_owned();
    if append_slash {
        source_str.push_str("/");
    }
    let mut command = process::Command::new(rsync);
    let command = command
        .arg("-a")
        .arg("--info=progress2")
        .arg(source_str)
        .stdout(process::Stdio::piped())
        .arg(dest.to_string_lossy().as_ref());
    let mut command = match command.spawn() {
        Ok(command) => command,
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => return Err(SyncError::RsyncNotFound(rsync.to_owned())),
            _ => return Err(err.into()),
        },
    };

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
fn update_archive_for_path<N>(
    relative_path: &Path,
    archive: &Archive,
    roots: &[PathBuf],
) -> Result<(), SyncError>
where
    N: NumRoots,
{
    let directory = relative_path.parent().unwrap();
    let mut archive_file = archive.for_directory(directory);
    let mut entries: ArchiveEntries<N> = archive_file.read()?;

    // remove old archive information (only needed when `relative_path` is a directory)
    {
        let replicas = entries.get(relative_path);
        if let Some(replicas) = replicas {
            let is_dir = any_directories_in(&replicas);

            if is_dir {
                debug!("There are descendant directories inside {:?} that need to be cleared from the archive", relative_path);
                let mut stack = Vec::new();
                stack.push(Archive::hash(relative_path));
                while let Some(item) = stack.pop() {
                    trace!(
                        "Scanning archive file {:?} for descendant directories",
                        item
                    );
                    let mut archive_file = archive.for_hashed_directory(item);
                    let entries: ArchiveEntries<N> = archive_file.read()?;

                    let dirs = entries
                        .iter()
                        .filter(|&(_, replicas)| any_directories_in(&replicas))
                        .map(|(hash, _)| *hash);
                    for dir in dirs {
                        stack.push(dir);
                    }

                    archive_file.remove_all()?;
                }
            } else {
                debug!("{:?} is not a directory, no pruning needed", relative_path);
            }
        } else {
            debug!(
                "No entry {:?} in archive {}, no pruning needed",
                relative_path, archive_file
            );
        }
    }

    info!("Updating {:?} in {}", relative_path, archive_file);

    // update archives for this exact path
    let replicas = ArchiveEntryPerReplica::from_roots::<N>(&roots, relative_path);
    entries.insert(relative_path, replicas);
    archive_file.write(&mut entries)?;

    // update archives for children of this path, only if it is a directory
    let first_root = roots[0].join(relative_path);
    if first_root.is_dir() {
        for entry in WalkDir::new(&first_root) {
            let entry = entry?;
            if entry.metadata()?.is_dir() {
                let dir_relative_path =
                    relative_path.join(entry.path().strip_prefix(&first_root).unwrap().as_os_str());
                let mut entries = ArchiveEntries::<N>::empty();

                for entry in entry.path().read_dir()? {
                    let entry = entry?;
                    if !entry.metadata()?.is_dir() {
                        let child_path = relative_path
                            .join(entry.path().strip_prefix(&first_root).unwrap().as_os_str());
                        let replicas = ArchiveEntryPerReplica::from_roots::<N>(&roots, &child_path);
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
    fn should_remove(&self, _: &Path) -> bool;

    /// return `SyncError::Cancelled` to cancel deleting the file,
    /// otherwise delete the file/move it to the trash.
    /// This must return an error if the file was not removed successfully.
    fn remove_file(&self, _: &Path) -> Result<(), SyncError>;

    /// Delete the directory and its contents
    /// This must return an error if the directory was not removed successfully.
    /// Ignoring errors will mean that  writes to the archive files when
    /// the replicas are still out of sync, resulting in an inconsistent state.
    fn remove_dir_all(&self, _: &Path) -> Result<(), SyncError>;
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
