use std::path::{Path, PathBuf};
use std::fs::{read_dir};
use std::os::unix::fs::MetadataExt;

use conflict::Conflict;
use archive;
use util::FnvHashMap;
use error::SyncError;
use state::{ArchiveEntryPerReplica, CurrentEntryPerReplica, SyncInfo};
use compare_files::file_contents_equal_cmd;

/// This mammoth function detects all conflicts between the two replicas.
/// It starts with the list of search directories provided and loops through them looking for conflicts.
/// If the recurse field of the SearchDirectories is set to true, then subdirectories inside each search directory will be added to the list.
pub fn find_conflicts<P: ProgressCallback>(archive: &archive::Archive, search: &mut SearchDirectories, config: &SyncInfo, progress_callback: &P) -> Result<(Vec<Conflict>, ConflictDetectionStatistics), SyncError> {
    let mut current_entries: FnvHashMap<PathBuf, Vec<CurrentEntryPerReplica>> = Default::default();
    let mut conflicts = ConflictList { list: Vec::new() };
    let mut stats = ConflictDetectionStatistics::new();
    let mut read_directories = 0;
    for root in &config.roots {
        if !root.exists() {
            return Err(SyncError::RootDoesntExist(root.to_path_buf()))
        }
    }
    search.directories.retain(|dir| !is_ignored(config, &dir));
    loop {
        let directory = match search.directories.pop() {
            Some(d) => d,
            None => break
        };

        if directory.is_absolute() {
            return Err(SyncError::AbsolutePathProvided(directory));
        }

        // get the previous entries (a snapshot of what it was like)
        let mut archive_for_directory = archive.for_directory(&directory);
        let mut archive_entries: archive::ArchiveEntries = try!(archive_for_directory.read()).into();

        // creates a list of all the different entries in the directory
        info!("Reading dir {:?}", directory);
        progress_callback.reading_directory(&directory, read_directories, search.directories.len());

        read_directories += 1;
        current_entries.clear();

        // when looking at the contents of this search directory, we must check if the
        // search directory itself is conflicting. if it is, then we will add it to the list
        // of paths to check.
        let mut current_search_directory_might_be_conflicting = false;

        for root in &config.roots {
            let absolute_directory = root.join(directory.clone());

            if absolute_directory.is_dir() {

                // loop through dir
                for archive_entry in try!(read_dir(absolute_directory)) {
                    let archive_entry = try!(archive_entry);
                    let relative_path = archive_entry.path();
                    let relative_path = relative_path.strip_prefix(root).unwrap_or_else(|_| panic!("couldn't strip prefix {:?} from {:?}", root, relative_path)).to_path_buf();

                    if is_ignored(config, &relative_path) {
                        trace!("Ignoring entry {:?}", relative_path);
                        continue;
                    }

                    trace!("Adding entry {:?}", relative_path);

                    current_entries.entry(relative_path).or_insert_with(|| {
                        Vec::new()
                    });
                }
            } else {
                current_search_directory_might_be_conflicting = true;
                info!("{:?} isn't a directory, might be conflicting", absolute_directory);
            }
        }

        if current_search_directory_might_be_conflicting {
            trace!("Adding entry {:?} (the search directory itself)", directory);
            current_entries.entry(directory.clone()).or_insert(Vec::new());
        }

        // analyses each item for conflicts
        debug!("Analysing items in {:?} for conflicts", directory);
        let current_entries_len = current_entries.len();
        'for_each_entry: for (i, (path, current_entry)) in current_entries.iter_mut().enumerate() {
            trace!("Analysing item {:?}", path);
            progress_callback.analysing_entry(&path, i, current_entries_len);

            for root in &config.roots {
                let absolute_path = root.join(path);
                let a = ArchiveEntryPerReplica::from(&*absolute_path);
                current_entry.push(CurrentEntryPerReplica { path: absolute_path, archive: a });
            }

            let mut keep_checking = true;
            if let Some(archive_entry) = archive_entries.get(path) {
                trace!("Checking archive files");
                let all_archive_files_valid = are_archive_files_fresh(archive_entry, current_entry);
                if all_archive_files_valid {
                    stats.archive_hits += 1;
                    keep_checking = false;
                }
            }

            if keep_checking {
                if let Some(conflict) = try!(do_further_analysis(path, &mut archive_entries, current_entry, &mut stats, config.compare_file_contents)) {
                    conflicts.add(conflict);
                    continue 'for_each_entry;
                }
            }

            // now we can assume that every replica contains an identical ArchiveEntry

            // we will recurse into the directory
            if let Some(last_replica) = current_entry.last() {
                if search.recurse && last_replica.path.is_dir() {
                    search.directories.push(path.clone());
                }
            }
        }

        if archive_entries.dirty {
            try!(archive_for_directory.write(archive_entries));
        }
    }

    Ok((conflicts.list, stats))
}

/// The list of directories to be searched.
#[derive(Debug, Clone)]
pub struct SearchDirectories {
    pub directories: Vec<PathBuf>,
    pub recurse: bool
}

impl SearchDirectories {
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

struct ConflictList {
    list: Vec<Conflict>
}

impl ConflictList {
    fn add(&mut self, conflict: Conflict) {
        let mut add = true;

        self.list.retain(|other| {
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
            self.list.push(conflict);
        }
    }
}

fn do_further_analysis(path: &Path, archive_entries: &mut archive::ArchiveEntries, current_entry: &Vec<CurrentEntryPerReplica>, stats: &mut ConflictDetectionStatistics, compare_file_contents: bool) -> Result<Option<Conflict>, SyncError> {
    trace!("Checking for incompatible entry types (eg: file vs folder vs empty)");
    // loop through 'abcdef' like: ab bc cd de ef
    for entry_window in current_entry.windows(2) {
        let equal_ty = ArchiveEntryPerReplica::equal_ty(&entry_window[0].archive, &entry_window[1].archive);
        if !equal_ty {
            info!("Conflict: types not equal");
            return Ok(Some(Conflict {
                path: path.to_path_buf(),
                previous_state: archive_entries.get(path).cloned(),
                current_state: current_entry.clone()
            }));
        }
    }

    trace!("Checking for different file sizes");
    for entry_window in current_entry.windows(2) {
        // if the sizes are different
        if entry_window[0].archive.is_file_or_symlink() && entry_window[1].archive.is_file_or_symlink() {
            let size_0 = try!(entry_window[0].path.metadata()).size();
            let size_1 = try!(entry_window[1].path.metadata()).size();
            if size_0 != size_1 {
                info!("Conflict: file sizes not equal: {} != {}", size_0, size_1);
                return Ok(Some(Conflict {
                    path: path.to_path_buf(),
                    previous_state: archive_entries.get(path).cloned(),
                    current_state: current_entry.clone()
                }));
            }
        }
    }

    // If they are both files, we will compare the contents
    if compare_file_contents {
        trace!("Checking file contents");
        for entry_window in current_entry.windows(2) {
            if entry_window[0].archive.is_file_or_symlink() && entry_window[1].archive.is_file_or_symlink() {
                if !try!(file_contents_equal_cmd(&entry_window[0].path, &entry_window[1].path)) {
                    info!("Conflict: file contents not equal");
                    return Ok(Some(Conflict {
                        path: path.to_path_buf(),
                        previous_state: archive_entries.get(path).cloned(),
                        current_state: current_entry.clone()
                    }));
                }
            }
        }
    }

    stats.archive_additions += 1;

    // since we now know that each ArchiveEntry is identical, we can store that information in the archive
    archive_entries.insert(path, current_entry.iter().map(|e| e.archive).collect());

    Ok(None)
}

#[derive(Debug)]
pub struct ConflictDetectionStatistics {
    pub archive_hits: usize,
    pub archive_additions: usize
}

impl ConflictDetectionStatistics {
    pub fn new() -> Self {
        ConflictDetectionStatistics {
            archive_hits: 0,
            archive_additions: 0
        }
    }
}


pub trait ProgressCallback {
    fn reading_directory(&self, path: &Path, checked: usize, remaining: usize);
    fn analysing_entry(&self, path: &Path, i: usize, total_in_directory: usize);
}

pub struct NoProgress;

impl ProgressCallback for NoProgress {
    fn reading_directory(&self, _: &Path, _: usize, _: usize) {}
    fn analysing_entry(&self, _: &Path, _: usize, _: usize) {}
}

/// checks that all the archive files for this path are identical
fn are_archive_files_fresh(previous: &Vec<ArchiveEntryPerReplica>, current: &Vec<CurrentEntryPerReplica>) -> bool {
    for (archive_entry, current_entry) in previous.iter().zip(current) {
        if archive_entry != &current_entry.archive {
            return false;
        }
    }
    true
}

/// checks if the path is on the ignore list
fn is_ignored(replica: &SyncInfo, path: &Path) -> bool {
    for ignore in &replica.ignore_path {
        //trace!("{:?} starts with {:?} = {}", path, ignore, path.starts_with(ignore));
        if path.starts_with(ignore) {
            return true;
        }
    }
    for ignore in &replica.ignore_regex {
        //trace!("{:?} is match {:?} = {}", ignore, path.to_str().unwrap(), ignore.is_match(path.to_str().unwrap()));
        if ignore.is_match(path.to_str().unwrap()) {
            return true;
        }
    }
    return false;
}
