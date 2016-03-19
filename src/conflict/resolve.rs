use conflict::Conflict;

pub fn guess(conflict: &Conflict) -> Option<usize> {
    info!("Looking at conflict {:?}", conflict.path);
    match conflict.previous_state {
        Some(ref previous_state) => {
            debug!("Previous state: {:?}", previous_state);
            let mut result = None;
            for (i, replica) in conflict.current_state.iter().enumerate() {
                // if it has changed on this replica
                if replica.archive != previous_state[i] {
                    debug!("{:?} != previous {:?}", replica.archive, previous_state[i]);
                    // we can't allow two conflicting files
                    if result.is_some() {

                        return None;
                    } else {
                        result = Some(i)
                    }
                }
            }
            result
        },
        None => {
            debug!("No previous state");
            let mut result = None;
            for (i, replica) in conflict.current_state.iter().enumerate() {
                if replica.archive.entry_exists() {
                    // we can't allow two conflicting files
                    if result.is_some() {
                        return None;
                    } else {
                        result = Some(i)
                    }
                }
            }
            result
        }
    }
}