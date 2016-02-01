use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::{read_dir};
use std::hash::Hasher;
use archive;
use util::{hash_single, FnvHashMap};
use error::SyncError;
use compare_files::file_contents_equal_cmd;
use std::os::unix::fs::MetadataExt;
use state::{HashedPath, ArchiveEntry, ArchiveEntryPerReplica, CurrentEntryPerReplica, SyncInfo};
use conflict::Conflict;

#[derive(Debug)]
pub struct SearchDirectories {
    pub directories: Vec<PathBuf>,
    recurse: bool
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


/// The mammoth function.
///
/// Detects all conflicts between the two replicas
pub fn find_conflicts<P: ProgressCallback>(archive: &archive::Archive, search: &mut SearchDirectories, config: &SyncInfo, progress_callback: &P) -> Result<(Vec<Conflict>, ConflictDetectionStatistics), SyncError> {
    let mut archive_entries = ArchiveEntries::new();
    let mut current_entries: FnvHashMap<PathBuf, Vec<CurrentEntryPerReplica>> = Default::default();
    let mut conflicts = Vec::new();
    let mut stats = ConflictDetectionStatistics::new();
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
        archive_entries.load_entries(try!(archive.get_entries_for_directory_or_empty(&directory)));

        // creates a list of all the different entries in the directory
        info!("Reading dir {:?}", directory);
        progress_callback.reading_directory(&directory);

        current_entries.clear();
        for root in &config.roots {
            let absolute_directory = root.join(directory.clone());

            if absolute_directory.is_dir() {

                // loop through dir
                for archive_entry in try!(read_dir(absolute_directory)) {
                    let archive_entry = try!(archive_entry);
                    let relative_path = archive_entry.path();
                    let relative_path = relative_path.strip_prefix(root).unwrap().to_path_buf();

                    trace!("Found entry {:?}", archive_entry.path());

                    if is_ignored(config, &relative_path) {
                        continue;
                    }

                    current_entries.entry(relative_path).or_insert(Vec::new());
                }
            } else {
                info!("{:?} doesn't exist", absolute_directory);
            }
        }

        // analyses each item for conflicts
        info!("Analysing for conflicts");
        let current_entries_len = current_entries.len();
        'for_each_entry: for (i, (path, entry)) in current_entries.iter_mut().enumerate() {
            debug!("Analysing {:?}", path);
            progress_callback.analysing_entry(&path, i, current_entries_len);

            for root in &config.roots {
                let absolute_path = root.join(path);
                let a = ArchiveEntryPerReplica::from(&*absolute_path);
                entry.push(CurrentEntryPerReplica { path: absolute_path, archive: a });
            }

            let mut do_further_analysis = true;
            if let Some(archive_entries) = archive_entries.get_for_path(path) {
                trace!("Checking archive files");
                let all_archive_files_valid = all_archive_files_valid(archive_entries, entry);
                if all_archive_files_valid {
                    stats.archive_hits += 1;
                    trace!("Archive files intact, no changes have been made");
                    do_further_analysis = false;
                }
            }

            if do_further_analysis {
                trace!("Checking for incompatible ArchiveEntry types");
                // loop through 'abcdef' like: ab bc cd de ef
                for entry_window in entry.windows(2) {
                    let equal_ty = ArchiveEntryPerReplica::equal_ty(&entry_window[0].archive, &entry_window[1].archive);
                    if !equal_ty {
                        info!("Conflict: Types not equal");
                        conflicts.push(Conflict {
                            path: path.clone(),
                            previous_state: archive_entries.get_for_path(path).cloned(),
                            current_state: entry.clone()
                        });
                        continue 'for_each_entry;
                    }
                }

                trace!("Checking for different file sizes");
                for entry_window in entry.windows(2) {
                    // if the sizes are different
                    if entry_window[0].archive.is_file_or_symlink() && entry_window[1].archive.is_file_or_symlink() {
                        let size_0 = try!(entry_window[0].path.metadata()).size();
                        let size_1 = try!(entry_window[1].path.metadata()).size();
                        if size_0 != size_1 {
                            info!("Conflict: File sizes not equal: {} != {}", size_0, size_1);
                            conflicts.push(Conflict {
                                path: path.clone(),
                                previous_state: archive_entries.get_for_path(path).cloned(),
                                current_state: entry.clone()
                            });
                            continue 'for_each_entry;
                        }
                    }
                }

                // If they are both files, we will compare the contents
                if config.compare_file_contents {
                    trace!("Checking file contents");
                    for entry_window in entry.windows(2) {
                        if entry_window[0].archive.is_file_or_symlink() && entry_window[1].archive.is_file_or_symlink() {
                            if !try!(file_contents_equal_cmd(&entry_window[0].path, &entry_window[1].path)) {
                                info!("Conflict: File contents not equal");
                                conflicts.push(Conflict {
                                    path: path.clone(),
                                    previous_state: archive_entries.get_for_path(path).cloned(),
                                    current_state: entry.clone()
                                });
                                continue 'for_each_entry;
                            }
                        }
                    }
                }

                stats.archive_additions += 1;
                // since we now know that each ArchiveEntry is identical, we can store that information in the archive
                archive_entries.insert(path, &entry.iter().map(|e| e.archive).collect());
            }

            // now we can assume that every replica contains an identical ArchiveEntry

            // we will recurse into the directory
            if let Some(last_replica) = entry.last() {
                if search.recurse && last_replica.path.is_dir() {
                    search.directories.push(path.clone());
                }
            }
        }

        if archive_entries.dirty {
            info!("Writing new archive files");
            try!(archive.write_entries_for_directory(&directory, archive_entries.get_entries_as_vec()));
        }
    }
    Ok((conflicts, stats))
}

#[derive(Debug)]
struct ArchiveEntries {
    entries: HashMap<HashedPath, Vec<ArchiveEntryPerReplica>>,
    dirty: bool
}

impl ArchiveEntries {
    fn new() -> Self {
        ArchiveEntries {
            entries: HashMap::new(),
            dirty: false
        }
    }

    fn load_entries(&mut self, entries_vec: Vec<ArchiveEntry>) {
        self.entries.clear();
        for e in entries_vec {
            self.entries.insert(e.path_hash/*Path::new(&e.path).to_path_buf()*/, e.replicas);
        };
        self.dirty = false;
    }

    fn get_entries_as_vec(&self) -> Vec<ArchiveEntry> {
        let mut entries_vec = Vec::new();
        for (hash, info) in &self.entries {
            entries_vec.push(ArchiveEntry::new(*hash, info.clone()));
        }
        entries_vec
    }

    fn get_for_path(&self, path: &Path) -> Option<&Vec<ArchiveEntryPerReplica>> {
        self.entries.get(&hash_single(path))
    }

    fn insert(&mut self, path: &Path, entries: &Vec<ArchiveEntryPerReplica>) {
        let hashed_path = hash_single(path);

        // warn because it means we are being inefficient
        warn!("Inserting data into archive for path {:?} (hashed: {})\n", path, hashed_path);
        self.entries.insert(hashed_path, entries.clone());
        self.dirty = true;
    }
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
    fn reading_directory(&self, &Path);
    fn analysing_entry(&self, &Path, usize, usize);
}

pub struct NoProgress;

impl ProgressCallback for NoProgress {
    fn reading_directory(&self, _: &Path) {}
    fn analysing_entry(&self, _: &Path, _: usize, _: usize) {}
}

fn all_archive_files_valid(previous: &Vec<ArchiveEntryPerReplica>, current: &Vec<CurrentEntryPerReplica>) -> bool {
    for (archive_entry, current_entry) in previous.iter().zip(current) {
        if archive_entry != &current_entry.archive {
            trace!("Archives differ");
            return false;
        }
    }
    true
}

fn is_ignored(replica: &SyncInfo, path: &Path) -> bool {
    for ignore in &replica.ignore_regex {
        if ignore.is_match(path.to_str().unwrap()) {
            return true;
        }
    }
    for ignore in &replica.ignore_path {
        if ignore == path.to_str().unwrap() {
            return true;
        }
    }
    return false;
}