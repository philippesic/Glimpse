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
    pending: std::collections::HashMap<u64, (u64, u64)>,
}
impl Checks {
    pub fn begin(&mut self, id: u64, generation: u64) -> u64 {
        self.next += 1;
        self.pending.insert(id, (self.next, generation));
        self.next
    }
    pub fn invalidate(&mut self, id: u64) {
        self.pending.remove(&id);
    }
    pub fn accept(&mut self, id: u64, token: u64, generation: u64) -> bool {
        if self
            .pending
            .get(&id)
            .is_some_and(|(pending, _)| *pending == token)
        {
            self.pending
                .remove(&id)
                .is_some_and(|(_, observed)| observed == generation)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod check_tests {
    use super::Checks;
    #[test]
    fn a_write_invalidates_reads_even_without_a_local_edit() {
        let mut checks = Checks::default();
        let old = checks.begin(4, 7);
        checks.invalidate(4);
        let after_save = checks.begin(4, 7);
        assert!(!checks.accept(4, old, 7));
        assert!(checks.accept(4, after_save, 7));
        assert!(!checks.accept(4, after_save, 7));
    }
    #[test]
    fn old_reads_and_edited_buffers_cannot_install_observations() {
        let mut checks = Checks::default();
        let old = checks.begin(2, 3);
        let current = checks.begin(2, 3);
        assert!(!checks.accept(2, old, 3));
        assert!(!checks.accept(2, current, 4));
        assert!(!checks.accept(2, current, 3));
    }
}
