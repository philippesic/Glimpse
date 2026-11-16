//! A close is committed only after its exact recovery generation is resolved.
use std::collections::BTreeMap;
#[derive(Clone)]
pub struct Pending {
    pub generation: u64,
    pub discard: bool,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct Closings {
    entries: BTreeMap<u64, Pending>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Ignored,
    Completed { discard: bool },
    Changed,
    Failed(String),
}
impl Closings {
    pub fn begin(&mut self, id: u64, generation: u64, discard: bool) -> bool {
        if self.entries.contains_key(&id) {
            return false;
        }
        self.entries.insert(
            id,
            Pending {
                generation,
                discard,
                error: None,
            },
        );
        true
    }
    pub fn contains(&self, id: u64) -> bool {
        self.entries.contains_key(&id)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn acknowledge(
        &mut self,
        id: u64,
        generation: u64,
        current: u64,
        result: Result<(), String>,
    ) -> Outcome {
        let Some(pending) = self
            .entries
            .get_mut(&id)
            .filter(|p| p.generation == generation)
        else {
            return Outcome::Ignored;
        };
        if current != generation {
            self.entries.remove(&id);
            return Outcome::Changed;
        }
        match result {
            Ok(()) => {
                let pending = self.entries.remove(&id).unwrap();
                Outcome::Completed {
                    discard: pending.discard,
                }
            }
            Err(error) => {
                pending.error = Some(error.clone());
                Outcome::Failed(error)
            }
        }
    }
    pub fn retry(&mut self) -> Vec<(u64, u64)> {
        self.entries
            .iter_mut()
            .filter_map(|(id, p)| p.error.take().map(|_| (*id, p.generation)))
            .collect()
    }
    pub fn cancel(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.entries).into_keys().collect()
    }
    pub fn failed(&self) -> Vec<(u64, bool)> {
        self.entries
            .iter()
            .filter(|(_, p)| p.error.is_some())
            .map(|(id, p)| (*id, p.discard))
            .collect()
    }
    pub fn forget(&mut self, id: u64) {
        self.entries.remove(&id);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discard_is_not_committed_on_failure_or_stale_ack() {
        let mut state = Closings::default();
        assert!(state.begin(7, 3, true));
        assert!(!state.begin(7, 4, false));
        assert_eq!(state.acknowledge(7, 2, 3, Ok(())), Outcome::Ignored);
        assert_eq!(
            state.acknowledge(7, 3, 3, Err("denied".into())),
            Outcome::Failed("denied".into())
        );
        assert!(state.contains(7));
        assert_eq!(state.retry(), vec![(7, 3)]);
        assert_eq!(
            state.acknowledge(7, 3, 3, Ok(())),
            Outcome::Completed { discard: true }
        );
        assert!(state.is_empty());
    }
    #[test]
    fn edits_during_cleanup_cancel_close() {
        let mut state = Closings::default();
        state.begin(2, 9, true);
        assert_eq!(state.acknowledge(2, 9, 10, Ok(())), Outcome::Changed);
        assert!(state.is_empty());
    }
    #[test]
    fn cancelled_close_ignores_late_ack() {
        let mut state = Closings::default();
        state.begin(1, 8, false);
        assert_eq!(state.cancel(), vec![1]);
        assert_eq!(state.acknowledge(1, 8, 9, Ok(())), Outcome::Ignored);
    }
}
