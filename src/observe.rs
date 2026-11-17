mod changes;
pub use changes::Changes;

use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Clone, Default, PartialEq, Eq)]
struct Watches {
    directory: PathBuf,
    documents: HashSet<PathBuf>,
}
impl Watches {
    fn resolve(&self) -> Resolved {
        let observed_directory =
            std::fs::canonicalize(&self.directory).unwrap_or_else(|_| self.directory.clone());
        let mut parents = HashSet::from([observed_directory.clone()]);
        let mut directories =
            std::collections::HashMap::from([(self.directory.clone(), observed_directory.clone())]);
        let documents = self
            .documents
            .iter()
            .map(|path| {
                let observed = if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
                {
                    let parent = directories.entry(parent.to_owned()).or_insert_with(|| {
                        std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_owned())
                    });
                    parents.insert(parent.clone());
                    parent.join(name)
                } else {
                    path.clone()
                };
                (observed, path.clone())
            })
            .collect();
        Resolved {
            directory: self.directory.clone(),
            observed_directory,
            documents,
            parents,
        }
    }
}
#[derive(Default)]
struct Resolved {
    directory: PathBuf,
    observed_directory: PathBuf,
    documents: Vec<(PathBuf, PathBuf)>,
    parents: HashSet<PathBuf>,
}
pub struct Observer {
    watches: Watches,
    latest: Arc<Mutex<Watches>>,
    force: Arc<AtomicBool>,
    jobs: mpsc::SyncSender<()>,
    changes: Arc<Mutex<Changes>>,
}
impl Observer {
    pub fn new(ctx: crate::wake::Context) -> Self {
        let (jobs, requests) = mpsc::sync_channel(1);
        let latest = Arc::new(Mutex::new(Watches::default()));
        let changes = Arc::new(Mutex::new(Changes::default()));
        let force = Arc::new(AtomicBool::new(false));
        let (requested, pending, forced) = (latest.clone(), changes.clone(), force.clone());
        thread::spawn(move || {
            let observed = Arc::new(Mutex::new(Resolved::default()));
            let make = || {
                let (events, interests, repaint) = (pending.clone(), observed.clone(), ctx.clone());
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    let mut change = Changes::default();
                    if !change.record(event) {
                        return;
                    }
                    let mapped = {
                        let current = interests.lock().unwrap();
                        change.mapped(
                            &current.observed_directory,
                            &current.directory,
                            &current.documents,
                        )
                    };
                    let Some(change) = mapped else {
                        return;
                    };
                    events.lock().unwrap().merge(change);
                    repaint.request_repaint();
                })
            };
            let mut watcher = make();
            let mut paths = HashSet::<PathBuf>::new();
            while requests.recv().is_ok() {
                let request = requested.lock().unwrap().clone();
                let resolved = request.resolve();
                let parents = resolved.parents.clone();
                *observed.lock().unwrap() = resolved;
                let mut error = None;
                if watcher.is_err() {
                    watcher = make();
                }
                match watcher.as_mut() {
                    Ok(watcher) => {
                        let force = forced.swap(false, Ordering::AcqRel);
                        let removed: Vec<_> = paths
                            .iter()
                            .filter(|path| force || !parents.contains(*path))
                            .cloned()
                            .collect();
                        for path in removed {
                            let _ = watcher.unwatch(&path);
                            paths.remove(&path);
                        }
                        let added: Vec<_> = parents.difference(&paths).cloned().collect();
                        for path in added {
                            match watcher.watch(&path, RecursiveMode::NonRecursive) {
                                Ok(()) => {
                                    paths.insert(path);
                                }
                                Err(failure) => {
                                    error.get_or_insert_with(|| {
                                        format!("{}: {failure}", crate::paths::display(&path))
                                    });
                                }
                            }
                        }
                    }
                    Err(failure) => {
                        error = Some(failure.to_string());
                    }
                }
                pending.lock().unwrap().registered(error);
                ctx.request_repaint();
            }
        });
        Self {
            watches: Watches::default(),
            latest,
            force,
            jobs,
            changes,
        }
    }
    pub fn update(&mut self, directory: PathBuf, documents: HashSet<PathBuf>) {
        let next = Watches {
            directory,
            documents,
        };
        if self.watches != next {
            self.watches = next.clone();
            *self.latest.lock().unwrap() = next;
            let _ = self.jobs.try_send(());
        }
    }
    pub fn rearm(&self) {
        self.force.store(true, Ordering::Release);
        let _ = self.jobs.try_send(());
    }
    pub fn changed(&self) -> Option<Changes> {
        let mut changes = self.changes.lock().unwrap();
        (!changes.empty()).then(|| std::mem::take(&mut *changes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn directory_aliases_share_one_watch_and_keep_logical_path_events() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        let alias = root.path().join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let resolved = Watches {
            directory: alias.clone(),
            documents: HashSet::from([alias.join("source.txt"), real.join("source.txt")]),
        }
        .resolve();
        assert_eq!(resolved.parents, HashSet::from([real.clone()]));
        assert_eq!(resolved.observed_directory, real);
        let mut created = Changes::default();
        created.record(Ok(notify::Event::new(notify::EventKind::Create(
            notify::event::CreateKind::File,
        ))
        .add_path(real.join("new.txt"))));
        assert!(
            created
                .mapped(
                    &resolved.observed_directory,
                    &resolved.directory,
                    &resolved.documents
                )
                .unwrap()
                .listing_changed(&alias)
        );
        let mut edited = Changes::default();
        edited.record(Ok(notify::Event::new(notify::EventKind::Modify(
            notify::event::ModifyKind::Data(notify::event::DataChange::Any),
        ))
        .add_path(real.join("source.txt"))));
        let mapped = edited
            .mapped(
                &resolved.observed_directory,
                &resolved.directory,
                &resolved.documents,
            )
            .unwrap();
        assert!(mapped.affects(&alias.join("source.txt")));
        assert!(mapped.affects(&real.join("source.txt")));
    }
    #[test]
    fn queued_notifications_coalesce_without_losing_errors() {
        let observer = Observer::new(crate::wake::Context::default());
        {
            let mut changes = observer.changes.lock().unwrap();
            changes.registered(None);
            changes.registered(Some("watch unavailable".into()));
            changes.registered(None);
        }
        assert_eq!(
            observer.changed().unwrap().error.as_deref(),
            Some("watch unavailable")
        );
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
