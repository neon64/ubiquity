pub mod detect;
pub mod resolve;

use std::path::PathBuf;
use state::{ArchiveEntryPerReplica, CurrentEntryPerReplica};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A conflict means that the files/folders differ.
/// There may be a suggested action to be taken.
pub struct Conflict {
    pub path: PathBuf,
    pub previous_state: Option<Vec<ArchiveEntryPerReplica>>,
    pub current_state: Vec<CurrentEntryPerReplica>
}