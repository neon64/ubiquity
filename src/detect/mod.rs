use std::path::{Path, PathBuf};
use generic_array::{GenericArray};

use error::SyncError;
use NumRoots;
use config::SyncInfo;
use archive::{Archive,ArchiveEntries};
use util::FnvHashMap;
use state::ArchiveEntryPerReplica;
use detect::util::*;
use detect::ext::is_item_in_sync;

mod ext;
mod util;

/// An instance of this struct represents the files/folders differ.
/// There may be a suggested action to be taken.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Difference<N: NumRoots> {
    /// The path at which the difference occurred
    pub path: PathBuf,
    /// The roots of the syncing operation
    pub roots: GenericArray<PathBuf, N>,
    /// The previous state that may be present from the archive
    pub previous_state: Option<GenericArray<ArchiveEntryPerReplica, N>>,
    /// The current state of the files
    pub current_state: GenericArray<ArchiveEntryPerReplica, N>
}

impl<N: NumRoots> Difference<N> {
    pub fn absolute_path_for_root(&self, index: usize) -> PathBuf {
        self.roots[index].join(&self.path)
    }
}

/// The result of update detection
pub struct DetectionResult<N: NumRoots> {
    pub differences: Vec<Difference<N>>,
    pub statistics: DetectionStatistics
}

impl<N: NumRoots> DetectionResult<N> {
    fn new() -> Self {
        DetectionResult {
            differences: Vec::new(),
            statistics: DetectionStatistics::new()
        }
    }

    fn add_difference(&mut self, conflict: Difference<N>) {
        let mut add = true;

        self.differences.retain(|other| {
            if other.path.starts_with(&conflict.path) {
                debug!("Removing nested conflict at {:?}", other.path);
                false
            } else if conflict.path.starts_with(&other.path) {
                debug!("Not adding nested conflict at {:?}", conflict.path);
                add = false;
                true
            } else {
                true
            }
        });

        if add {
            self.differences.push(conflict);
        }
    }
}

/// The list of directories to be searched.
#[derive(Debug, Clone)]
pub struct SearchDirectories {
    pub directories: Vec<PathBuf>,
    pub recurse: bool
}

impl SearchDirectories {
    /// Builds a value that searches everything inside the root.
    pub fn from_root() -> SearchDirectories {
        SearchDirectories {
            directories: vec![Path::new("").to_path_buf()],
            recurse: true
        }
    }

    pub fn new(directories: Vec<PathBuf>, recurse: bool) -> Self {
        SearchDirectories {
            directories: directories,
            recurse: recurse
        }
    }
}


#[derive(Debug)]
/// Basic statistics about the accuracy of archives during the detection process.
pub struct DetectionStatistics {
    /// The number of times the archives were up to date and reported no change.
    pub archive_hits: usize,
    /// The number of times the archives had to be added to.
    pub archive_additions: usize
}

impl DetectionStatistics {
    pub fn new() -> Self {
        DetectionStatistics {
            archive_hits: 0,
            archive_additions: 0_
        }
    }
}

/// Progress reporting for the update detection process.
pub trait ProgressCallback {
    /// Called when a new directory is being searched.
    fn reading_directory(&self, path: &Path, checked: usize, remaining: usize);
}

/// An empty implementation of `ProgressCallback`
pub struct EmptyProgressCallback;

impl ProgressCallback for EmptyProgressCallback {
    fn reading_directory(&self, _: &Path, _: usize, _: usize) {}
}







/// This mammoth function detects all differences between the two replicas,
/// inside the provided search directories.
///
/// If the `recurse` field of `SearchDirectories` is true, then all subdirectories will be added
/// to the list. If `recurse` is false, then just the items in that directory will be considered.
///
/// The archive is used to speed up update detection by comparing  the `ino` and `ctime` properties of a file/directory to a previously known value, instead of directly comparing file contents accross replicas.
pub fn find_updates<N, P>(archive: &Archive, search: &mut SearchDirectories, config: &SyncInfo<N>, progress_callback: &P) -> Result<DetectionResult<N>, SyncError> where N: NumRoots, P: ProgressCallback {
    // this is used to keep track of the current items in the current search directory
    let mut current_entries: FnvHashMap<PathBuf, GenericArray<ArchiveEntryPerReplica, N>> = Default::default();
    let mut result = DetectionResult::new();
    let mut read_directories = 0;

    // warn about non-existent roots early in the processes
    check_all_roots_exist(config.roots.iter())?;

    search.directories.retain(|dir| !is_ignored(&config.ignore, &dir));

    loop {
        current_entries.clear();

        let sd = match search.directories.pop() {
            Some(d) => d,
            None => break
        };

        if sd.is_absolute() {
            return Err(SyncError::AbsolutePathProvided(sd));
        }

        // creates a list of all the different entries in the directory
        debug!("Reading dir {:?}", sd);
        progress_callback.reading_directory(&sd, read_directories, search.directories.len());
        read_directories += 1;

        // get the previous entries (a snapshot of what it was like)
        let mut sd_archive_file = archive.for_directory(&sd);
        let mut sd_archive_entries: ArchiveEntries<N> = sd_archive_file.read()?.into();

        // scan the directory contents accross all replicas, adding items to check to `current_entries`
        scan_directory_contents(&sd, &mut current_entries, config)?;

        // analyses each item in this directory
        debug!("Analysing items in {:?}", sd);
        for (path, current_entry) in current_entries.iter_mut() {
            let mut keep_checking = true;
            if let Some(archive_entry) = sd_archive_entries.get(path) {
                trace!("Checking archive files");
                if are_archive_files_identical(archive_entry, current_entry) {
                    result.statistics.archive_hits += 1;
                    keep_checking = false;
                }
            }

            if keep_checking {
                if is_item_in_sync(path, current_entry, config.compare_file_contents, &config.roots)? {
                    // This item is identical, let's store that in the archive for next time
                    sd_archive_entries.insert(path, current_entry.clone());
                    result.statistics.archive_additions += 1;
                } else {
                    // the Difference struct encapsulates everything needed to resolve
                    // a conflict independently of any other information.
                    let difference = Difference {
                        path: path.to_path_buf(),
                        roots: config.roots.clone(),
                        previous_state: sd_archive_entries.get(path).cloned(),
                        current_state: current_entry.clone()
                    };
                    result.add_difference(difference);
                    continue;
                }
            }

            // This item is identical on every replica so if it is a directory we
            // will start looking inside its contents, as long as the user requested it with
            // the SearchDirectories.recurse option
            if let Some(root) = config.roots.last() {
                if search.recurse && root.join(path).is_dir() {
                    search.directories.push(path.clone());
                }
            }
        }

        if sd_archive_entries.is_dirty() {
            sd_archive_file.write(&mut sd_archive_entries)?;
        }
    }

    Ok(result)
}
