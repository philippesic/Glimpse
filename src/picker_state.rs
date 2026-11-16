use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Completion {
    pending: Option<PathBuf>,
    finished: bool,
}
impl Completion {
    pub fn begin(&mut self, path: PathBuf) -> bool {
        if self.finished || self.pending.is_some() {
            return false;
        }
        self.pending = Some(path);
        true
    }
    pub fn complete(&mut self, path: &Path, valid: bool) -> bool {
        if self.finished || self.pending.as_deref() != Some(path) {
            return false;
        }
        self.pending = None;
        if valid {
            self.finished = true;
        }
        valid
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        self.finished = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_wins_over_an_in_flight_validation_and_cannot_restart() {
        let mut state = Completion::default();
        assert!(state.begin("file".into()));
        state.cancel();
        assert!(!state.complete(Path::new("file"), true));
        assert!(!state.begin("file".into()));
    }
    #[test]
    fn only_the_pending_path_can_finish_and_failed_validation_can_retry() {
        let mut state = Completion::default();
        assert!(state.begin("file".into()));
        assert!(!state.begin("other".into()));
        assert!(!state.complete(Path::new("other"), true));
        assert!(!state.complete(Path::new("file"), false));
        assert!(state.begin("file".into()));
        assert!(state.complete(Path::new("file"), true));
        assert!(!state.complete(Path::new("file"), true));
    }
}
