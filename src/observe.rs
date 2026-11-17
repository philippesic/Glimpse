use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};
pub struct Observer {
    paths: HashSet<PathBuf>,
    latest: Arc<Mutex<HashSet<PathBuf>>>,
    jobs: mpsc::SyncSender<()>,
    changes: mpsc::Receiver<Result<(), String>>,
}
impl Observer {
    pub fn new(ctx: crate::wake::Context) -> Self {
        let (jobs, requests) = mpsc::sync_channel(1);
        let (tx, changes) = mpsc::sync_channel(8);
        let latest = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
        let pending = latest.clone();
        thread::spawn(move || {
            let events = tx.clone();
            let repaint = ctx.clone();
            let mut watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.as_ref().is_ok_and(|event| {
                        matches!(
                            event.kind,
                            notify::EventKind::Access(_) | notify::EventKind::Other
                        )
                    }) {
                        return;
                    }
                    if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() {
                        eprintln!("WATCH_EVENT {:?}", event.as_ref().map(|event| &event.kind));
                    }
                    if events
                        .try_send(event.map(|_| ()).map_err(|e| e.to_string()))
                        .is_ok()
                    {
                        repaint.request_repaint();
                    }
                });
            let mut paths = HashSet::new();
            while requests.recv().is_ok() {
                let next = pending.lock().unwrap().clone();
                let result = (|| {
                    let watcher = watcher.as_mut().map_err(|e| e.to_string())?;
                    for path in paths.difference(&next) {
                        let _ = watcher.unwatch(path);
                    }
                    for path in next.difference(&paths) {
                        watcher
                            .watch(path, RecursiveMode::NonRecursive)
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })();
                paths = next;
                if result.is_err() {
                    let _ = tx.try_send(result);
                    ctx.request_repaint();
                }
            }
        });
        Self {
            paths: HashSet::new(),
            latest,
            jobs,
            changes,
        }
    }
    pub fn update(&mut self, parents: HashSet<PathBuf>) {
        if self.paths != parents {
            self.paths = parents.clone();
            *self.latest.lock().unwrap() = parents;
            let _ = self.jobs.try_send(());
        }
    }
    pub fn changed(&self) -> Option<Result<(), String>> {
        let mut result = None;
        while let Ok(change) = self.changes.try_recv() {
            if result
                .as_ref()
                .is_none_or(|value: &Result<(), String>| value.is_ok())
            {
                result = Some(change);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_notifications_coalesce_without_losing_errors() {
        let (jobs, _) = mpsc::sync_channel(1);
        let (events, changes) = mpsc::sync_channel(8);
        let observer = Observer {
            paths: HashSet::new(),
            latest: Arc::new(Mutex::new(HashSet::new())),
            jobs,
            changes,
        };
        for change in [Ok(()), Err("watch unavailable".into()), Ok(())] {
            events.send(change).unwrap();
        }
        assert_eq!(observer.changed(), Some(Err("watch unavailable".into())));
        assert!(observer.changed().is_none());
    }
}

#[derive(Default)]
pub struct Checks {
    next: u64,
    pending: std::collections::HashMap<u64, Pending>,
    waiting: HashSet<u64>,
    queue: std::collections::VecDeque<u64>,
}
struct Pending {
    token: u64,
    generation: u64,
    valid: bool,
}
impl Checks {
    pub fn request(&mut self, id: u64) {
        if self.waiting.insert(id) {
            self.queue.push_back(id);
        }
    }
    pub fn next(&mut self, mut generation: impl FnMut(u64) -> Option<u64>) -> Option<(u64, u64)> {
        if self.pending.len() >= 2 {
            return None;
        }
        for _ in 0..self.queue.len() {
            let id = self.queue.pop_front()?;
            if self.pending.contains_key(&id) {
                self.queue.push_back(id);
                continue;
            }
            self.waiting.remove(&id);
            let Some(generation) = generation(id) else {
                continue;
            };
            self.next += 1;
            self.pending.insert(
                id,
                Pending {
                    token: self.next,
                    generation,
                    valid: true,
                },
            );
            return Some((id, self.next));
        }
        None
    }
    pub fn invalidate(&mut self, id: u64) {
        if let Some(pending) = self.pending.get_mut(&id) {
            pending.valid = false;
        }
        self.waiting.remove(&id);
        self.queue.retain(|queued| *queued != id);
    }
    pub fn complete(&mut self, id: u64, token: u64, generation: Option<u64>) -> bool {
        if self
            .pending
            .get(&id)
            .is_none_or(|pending| pending.token != token)
        {
            return false;
        }
        let pending = self.pending.remove(&id).unwrap();
        let Some(generation) = generation else {
            self.invalidate(id);
            return false;
        };
        let again = self.waiting.contains(&id);
        if pending.valid && pending.generation == generation && !again {
            return true;
        }
        if pending.valid || again {
            self.request(id);
        }
        false
    }
    pub fn retry(&mut self, id: u64, token: u64) {
        if self
            .pending
            .get(&id)
            .is_some_and(|pending| pending.token == token)
        {
            self.pending.remove(&id);
            self.request(id);
        }
    }
}

#[cfg(test)]
mod check_tests {
    use super::Checks;
    #[test]
    fn a_write_invalidates_reads_even_without_a_local_edit() {
        let mut checks = Checks::default();
        checks.request(4);
        let (_, old) = checks.next(|_| Some(7)).unwrap();
        checks.invalidate(4);
        checks.request(4);
        assert!(checks.next(|_| Some(7)).is_none());
        assert!(!checks.complete(4, old, Some(7)));
        let (_, after_save) = checks.next(|_| Some(7)).unwrap();
        assert!(checks.complete(4, after_save, Some(7)));
        assert!(!checks.complete(4, after_save, Some(7)));
    }
    #[test]
    fn old_reads_and_edited_buffers_cannot_install_observations() {
        let mut checks = Checks::default();
        checks.request(2);
        let (_, old) = checks.next(|_| Some(3)).unwrap();
        checks.request(2);
        assert!(checks.next(|_| Some(4)).is_none());
        assert!(!checks.complete(2, old, Some(4)));
        let (_, current) = checks.next(|_| Some(4)).unwrap();
        assert!(!checks.complete(2, old, Some(4)));
        assert!(checks.complete(2, current, Some(4)));
        assert!(!checks.complete(2, current, Some(4)));
    }
    #[test]
    fn quiet_local_edits_retry_and_notifications_coalesce_with_fairness() {
        let mut checks = Checks::default();
        for id in 1..=3 {
            checks.request(id);
        }
        let (_, first) = checks.next(|_| Some(0)).unwrap();
        let (_, second) = checks.next(|_| Some(0)).unwrap();
        assert!(checks.next(|_| Some(0)).is_none());
        for _ in 0..1000 {
            checks.request(1);
        }
        assert_eq!(checks.queue.len(), 2);
        assert_eq!(checks.pending.len(), 2);
        assert!(!checks.complete(1, first, Some(1000)));
        let (id, third) = checks.next(|_| Some(0)).unwrap();
        assert_eq!(id, 3);
        assert!(checks.complete(2, second, Some(0)));
        let (id, latest) = checks.next(|_| Some(1000)).unwrap();
        assert_eq!(id, 1);
        assert!(checks.complete(3, third, Some(0)));
        assert!(checks.complete(1, latest, Some(1000)));
        checks.request(4);
        let (_, token) = checks.next(|_| Some(0)).unwrap();
        assert!(!checks.complete(4, token, Some(1)));
        let (_, retry) = checks.next(|_| Some(1)).unwrap();
        assert!(checks.complete(4, retry, Some(1)));
    }
    #[test]
    fn closed_documents_and_deferred_old_tokens_cannot_resurrect_work() {
        let mut checks = Checks::default();
        checks.request(1);
        let (_, token) = checks.next(|_| Some(0)).unwrap();
        checks.invalidate(1);
        assert!(!checks.complete(1, token, None));
        checks.retry(1, token);
        assert!(checks.next(|_| Some(0)).is_none());
        checks.request(2);
        let (_, deferred) = checks.next(|_| Some(0)).unwrap();
        checks.retry(2, deferred);
        let (_, current) = checks.next(|_| Some(1)).unwrap();
        checks.retry(2, deferred);
        assert!(checks.complete(2, current, Some(1)));
    }
}
