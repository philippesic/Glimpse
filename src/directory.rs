#[derive(Clone)]
pub struct Entry {
    pub path: std::path::PathBuf,
    pub directory: bool,
    pub symlink: bool,
}
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Batch {
    pub path: PathBuf,
    pub generation: u64,
    pub entries: Vec<Entry>,
    pub done: bool,
    pub error: Option<String>,
}
struct Request {
    path: PathBuf,
    generation: u64,
}
pub struct Directories {
    jobs: Option<mpsc::SyncSender<Request>>,
    pub replies: mpsc::Receiver<Batch>,
    latest: Arc<Mutex<HashMap<PathBuf, u64>>>,
    next: AtomicU64,
    stop: Arc<AtomicBool>,
}
impl Directories {
    pub fn new(ctx: crate::wake::Context) -> Self {
        let (jobs, requests) = mpsc::sync_channel::<Request>(64);
        let requests = Arc::new(Mutex::new(requests));
        let (results, replies) = mpsc::sync_channel(16);
        let latest = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        for _ in 0..2 {
            let (requests, results, latest, stop, ctx) = (
                requests.clone(),
                results.clone(),
                latest.clone(),
                stop.clone(),
                ctx.clone(),
            );
            thread::spawn(move || {
                loop {
                    let Ok(request) = requests.lock().unwrap().recv() else {
                        break;
                    };
                    let cancelled = || {
                        stop.load(Ordering::Relaxed)
                            || latest.lock().unwrap().get(&request.path)
                                != Some(&request.generation)
                    };
                    if cancelled() {
                        continue;
                    }
                    let send = |entries, done, error| {
                        let mut batch = Batch {
                            path: request.path.clone(),
                            generation: request.generation,
                            entries,
                            done,
                            error,
                        };
                        loop {
                            if cancelled() {
                                return false;
                            }
                            match results.try_send(batch) {
                                Ok(()) => {
                                    ctx.request_repaint();
                                    return true;
                                }
                                Err(mpsc::TrySendError::Disconnected(_)) => return false,
                                Err(mpsc::TrySendError::Full(value)) => {
                                    batch = value;
                                    thread::sleep(Duration::from_millis(2));
                                }
                            }
                        }
                    };
                    #[cfg(feature = "test-support")]
                    crate::metrics::delay("DIRECTORY");
                    let listing = match std::fs::read_dir(&request.path) {
                        Ok(value) => value,
                        Err(error) => {
                            send(vec![], true, Some(error.to_string()));
                            continue;
                        }
                    };
                    let mut batch = Vec::new();
                    let mut bytes = 0;
                    let mut count = 0;
                    let mut error = None;
                    let mut last = Instant::now();
                    for item in listing {
                        if cancelled() {
                            break;
                        }
                        let item = match item {
                            Ok(item) => item,
                            Err(e) => {
                                error = Some(e.to_string());
                                break;
                            }
                        };
                        let kind = match item.file_type() {
                            Ok(kind) => kind,
                            Err(e) => {
                                error = Some(e.to_string());
                                break;
                            }
                        };
                        let path = item.path();
                        bytes += path.as_os_str().len() * 2 + std::mem::size_of::<Entry>();
                        count += 1;
                        if count > 100_000 || bytes > 16 * 1024 * 1024 {
                            error = Some(
                                "Partial — directory entry or 16 MiB listing budget reached".into(),
                            );
                            break;
                        }
                        batch.push(Entry {
                            path,
                            directory: kind.is_dir(),
                            symlink: kind.is_symlink(),
                        });
                        if batch.len() >= if count <= 256 { 256 } else { 2048 }
                            || last.elapsed() >= Duration::from_millis(20)
                        {
                            if !send(std::mem::take(&mut batch), false, None) {
                                break;
                            }
                            last = Instant::now();
                        }
                    }
                    if !cancelled() {
                        send(batch, true, error);
                    }
                }
            });
        }
        Self {
            jobs: Some(jobs),
            replies,
            latest,
            next: AtomicU64::new(1),
            stop,
        }
    }
    pub fn request(&self, path: PathBuf) -> Result<u64, String> {
        let generation = self.next.fetch_add(1, Ordering::Relaxed);
        self.latest.lock().unwrap().insert(path.clone(), generation);
        self.jobs
            .as_ref()
            .ok_or("Directory service stopped")?
            .try_send(Request {
                path: path.clone(),
                generation,
            })
            .map_err(|_| {
                self.latest.lock().unwrap().remove(&path);
                "Directory queue full — retry this folder".to_owned()
            })?;
        Ok(generation)
    }
    pub fn retain(&self, paths: &[PathBuf]) {
        self.latest
            .lock()
            .unwrap()
            .retain(|path, _| paths.contains(path));
    }
}
impl Drop for Directories {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.jobs.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_and_rejects_cancelled_generations() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..600 {
            std::fs::write(root.path().join(format!("entry-{i}")), "").unwrap();
        }
        let service = Directories::new(crate::wake::Context::default());
        let path = root.path().to_owned();
        let old = service.request(path.clone()).unwrap();
        let current = service.request(path).unwrap();
        assert!(current > old);
        let mut count = 0;
        let mut batches = 0;
        loop {
            let batch = service
                .replies
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if batch.generation != current {
                continue;
            }
            count += batch.entries.len();
            batches += 1;
            if batch.done {
                break;
            }
        }
        assert_eq!(count, 600);
        assert!(batches >= 2);
    }
}
