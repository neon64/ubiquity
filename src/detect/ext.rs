use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use generic_array::{GenericArray};

use state::ArchiveEntryPerReplica;
use error::SyncError;
use compare_files::file_contents_equal_cmd;
use NumRoots;

pub fn is_item_in_sync<N: NumRoots>(
    path: &Path,
    current_entry: &GenericArray<ArchiveEntryPerReplica, N>,
    compare_file_contents: bool,
    roots: &GenericArray<PathBuf, N>
) -> Result<bool, SyncError> {

    trace!("Checking for incompatible entry types (eg: file vs folder vs empty)");
    // loop through 'abcdef' like: ab bc cd de ef
    for entry_window in current_entry.windows(2) {
        let equal_ty = ArchiveEntryPerReplica::equal_ty(&entry_window[0], &entry_window[1]);
        if !equal_ty {
            warn!("Difference at {:?} - types not equal", path);
            return Ok(false)
        }
    }

    trace!("Checking for different file sizes");
    for (entry_window, roots) in current_entry.windows(2).zip(roots.windows(2)) {
        // if the sizes are different
        if entry_window[0].is_file_or_symlink() && entry_window[1].is_file_or_symlink() {
            let size_0 = roots[0].join(path).metadata()?.size();
            let size_1 = roots[1].join(path).metadata()?.size();
            if size_0 != size_1 {
                warn!("Difference at path {:?} - file sizes not equal: {} != {}", path, size_0, size_1);
                return Ok(false)
            }
        }
    }

    // If they are both files, we will compare the contents
    if compare_file_contents {
        trace!("Checking file contents");
        for (entry_window, roots) in current_entry.windows(2).zip(roots.windows(2)) {
            if entry_window[0].is_file_or_symlink() && entry_window[1].is_file_or_symlink() {
                if !file_contents_equal_cmd(&roots[0].join(path), &roots[1].join(path))? {
                    warn!("Difference at path {:?} - file contents not equal", path);
                    return Ok(false)
                }
            }
        }
    }

    Ok(true)
}