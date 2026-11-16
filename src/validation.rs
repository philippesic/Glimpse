use std::collections::BTreeMap;

#[derive(Default)]
pub struct Validations {
    next: u64,
    active: Option<Batch>,
}
struct Batch {
    attempt: u64,
    snapshots: BTreeMap<u64, u64>,
    pending: BTreeMap<u64, u64>,
}
impl Validations {
    pub fn begin(&mut self, snapshots: impl IntoIterator<Item = (u64, u64)>) -> u64 {
        self.next += 1;
        let snapshots: BTreeMap<_, _> = snapshots.into_iter().collect();
        self.active = Some(Batch {
            attempt: self.next,
            pending: snapshots.clone(),
            snapshots,
        });
        self.next
    }
    pub fn invalidate_document(&mut self, id: u64) -> bool {
        let included = self
            .active
            .as_ref()
            .is_some_and(|batch| batch.snapshots.contains_key(&id));
        if included {
            self.invalidate();
        }
        included
    }
    pub fn invalidate(&mut self) {
        self.active = None;
    }
    pub fn pending(&self) -> bool {
        self.active.is_some()
    }
    pub fn accepts(&self, attempt: u64, id: u64, generation: u64) -> bool {
        self.active.as_ref().is_some_and(|batch| {
            batch.attempt == attempt && batch.pending.get(&id) == Some(&generation)
        })
    }
    pub fn acknowledge(
        &mut self,
        attempt: u64,
        id: u64,
        generation: u64,
        result: Result<(), String>,
    ) -> Result<Option<Vec<(u64, u64)>>, String> {
        let Some(batch) = self.active.as_mut().filter(|batch| {
            batch.attempt == attempt && batch.pending.get(&id) == Some(&generation)
        }) else {
            return Ok(None);
        };
        if let Err(error) = result {
            self.active = None;
            return Err(error);
        }
        batch.pending.remove(&id);
        if batch.pending.is_empty() {
            Ok(Some(
                self.active.take().unwrap().snapshots.into_iter().collect(),
            ))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Validations;
    #[test]
    fn unrelated_edits_do_not_cancel_requested_validation() {
        let mut state = Validations::default();
        let attempt = state.begin([(1, 3)]);
        assert!(!state.invalidate_document(2));
        assert_eq!(
            state.acknowledge(attempt, 1, 3, Ok(())).unwrap(),
            Some(vec![(1, 3)])
        );
        state.begin([(1, 4)]);
        assert!(state.invalidate_document(1));
        assert!(!state.pending());
    }
    #[test]
    fn an_edit_cancels_every_acknowledgement_from_the_attempt() {
        let mut state = Validations::default();
        let attempt = state.begin([(1, 8), (2, 3)]);
        assert_eq!(state.acknowledge(attempt, 1, 8, Ok(())).unwrap(), None);
        state.invalidate();
        assert_eq!(state.acknowledge(attempt, 2, 3, Ok(())).unwrap(), None);
        assert!(!state.pending());
    }
    #[test]
    fn retries_reject_previous_attempts_and_duplicate_replies() {
        let mut state = Validations::default();
        let old = state.begin([(1, 8), (2, 3)]);
        let new = state.begin([(1, 8), (2, 3)]);
        assert_eq!(state.acknowledge(old, 1, 8, Ok(())).unwrap(), None);
        assert_eq!(state.acknowledge(new, 2, 3, Ok(())).unwrap(), None);
        assert_eq!(state.acknowledge(new, 2, 3, Ok(())).unwrap(), None);
        assert_eq!(
            state.acknowledge(new, 1, 8, Ok(())).unwrap(),
            Some(vec![(1, 8), (2, 3)])
        );
    }
    #[test]
    fn failure_ends_a_batch_and_stale_failures_cannot_cancel_a_retry() {
        let mut state = Validations::default();
        let old = state.begin([(4, 1), (5, 1)]);
        assert!(state.acknowledge(old, 4, 1, Err("changed".into())).is_err());
        let new = state.begin([(4, 2)]);
        assert_eq!(
            state.acknowledge(old, 5, 1, Err("late".into())).unwrap(),
            None
        );
        assert_eq!(state.acknowledge(new, 4, 1, Ok(())).unwrap(), None);
        assert_eq!(
            state.acknowledge(new, 4, 2, Ok(())).unwrap(),
            Some(vec![(4, 2)])
        );
    }
}
