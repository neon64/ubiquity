use detect::Difference;
use NumRoots;
use ReplicaIndex;

#[derive(Debug, Copy, Clone)]
/// The result of `reconciliation`.
pub enum Operation {
    /// the provided replica was correct
    PropagateFromMaster(ReplicaIndex),
    /// the item was changed on multiple replicas and we don't know which
    ItemChangedOnMultipleReplicas,
    /// the item differs, but there was no previous state in the archives so we don't know which replica is 'correct'
    ItemDiffersBetweenReplicasAndNoArchive
}

/// Determines which replica (if any), has the most up-to-date copy of the item.
pub fn guess_operation<N: NumRoots>(difference: &Difference<N>) -> Operation {
    info!("Reconciling difference at {:?}", difference.path);
    match difference.previous_state {
        Some(ref previous_state) => {
            debug!("Previous state: {:?}", previous_state);

            // this initial value is not strictly true, but it should always be overwritten inside the following loop
            let mut result = Operation::ItemChangedOnMultipleReplicas;

            for (i, replica) in difference.current_state.iter().enumerate() {
                // if it has changed on this replica
                if replica != &previous_state[i] {

                    debug!("Item was changed in replica {}: was {:?}, now {:?}", i, previous_state[i], replica);
                    if let Operation::PropagateFromMaster(_) = result {
                        // it has changed on multiple replicas so we don't know which one is correct
                        return Operation::ItemChangedOnMultipleReplicas;
                    } else {
                        // this is (so far) the 'master' replica
                        result = Operation::PropagateFromMaster(i);
                    }
                }
            }
            result
        },
        None => {
            debug!("No previous state from the archive");
            let mut result = Operation::ItemDiffersBetweenReplicasAndNoArchive;
            for (i, replica) in difference.current_state.iter().enumerate() {
                if replica.entry_exists() {
                    // we can't allow two conflicting files
                    if let Operation::PropagateFromMaster(_) = result {
                        // it has changed on multiple replicas so we don't know which one is correct
                        return Operation::ItemDiffersBetweenReplicasAndNoArchive;
                    } else {
                        // this is (so far) the 'master' replica
                        result = Operation::PropagateFromMaster(i);
                    }
                }
            }
            result
        }
    }
}