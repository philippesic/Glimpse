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
    pub fn new(ctx: eframe::egui::Context) -> Self {
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
