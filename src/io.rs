use crate::wake;
use crate::{document::DiskFile, fileops, storage};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};
pub enum Job {
    #[cfg(test)]
    Hold(mpsc::Sender<()>, mpsc::Receiver<()>),
    #[cfg(test)]
    HoldMaintenance(mpsc::Sender<()>, mpsc::Receiver<()>),
    Visit(PathBuf),
    RecentLocation(Vec<PathBuf>),
    External(PathBuf, bool),
    Create(PathBuf, bool, String),
    Rename(PathBuf, PathBuf),
    Trash(PathBuf),
    TrashRecord(PathBuf),
    Prepare(SaveDestination),
    SaveTo(SaveDestination),
    Recover(u64, storage::Draft),
    Wake,
    Inventory,
    RestoreHistory(u64, PathBuf),
    RestoreObserved(u64, PathBuf),
    Import(u64, PathBuf),
    ClearThrough(u64, u64),
    Settings(storage::Settings),
    LoadSettings,
    Check(u64, PathBuf, u64),
    Finish(u64, u64, DiskFile, Vec<PathBuf>),
    Choose(PathBuf, bool),
    Open(PathBuf, u64, bool),
    Save {
        id: u64,
        disk: DiskFile,
        text: String,
        generation: u64,
        without_recovery: bool,
    },
}
#[derive(Clone)]
pub struct SaveDestination {
    pub id: u64,
    pub path: PathBuf,
    pub text: String,
    pub copy: bool,
    pub bom: bool,
    pub generation: u64,
    pub expected: Option<DiskFile>,
    pub original: DiskFile,
    pub without_recovery: bool,
}
pub enum Reply {
    RecentLocation(Option<PathBuf>),
    SettingsLoaded(Result<storage::Settings, String>),
    Changed(Result<(), String>),
    Records(Result<storage::Inventory, String>),
    Finished(u64, u64, Result<(), String>),
    Chosen(PathBuf, Result<(), String>),
    OpenDirectory(PathBuf, u64),
    HistoryWarning(String),
    Cleared(u64, u64, Result<(), String>),
    RecordTrashed(Result<(), String>),
    Prepared(Box<SaveDestination>, Result<Option<DiskFile>, String>),
    Imported(u64, Result<(storage::Draft, DiskFile), String>),
    Done(Result<(), String>),
    Visit(PathBuf, bool),
    SavedTo(Box<SaveDestination>, Result<DiskFile, String>),
    Checked(u64, u64, Result<(DiskFile, String), String>),
    Open(PathBuf, u64, Result<(DiskFile, String), String>),
    Save(u64, String, Result<DiskFile, String>),
}

pub struct IoWorker {
    jobs: Option<mpsc::SyncSender<Job>>,
    reads: Option<mpsc::SyncSender<Job>>,
    maintenance: Option<mpsc::SyncSender<Job>>,
    pub replies: mpsc::Receiver<Reply>,
    errors: mpsc::Sender<Reply>,
    drafts: Arc<Mutex<HashMap<u64, storage::Draft>>>,
    handles: Vec<thread::JoinHandle<()>>,
    clears: Arc<Mutex<HashMap<u64, u64>>>,
    cleared_through: Arc<Mutex<HashMap<u64, u64>>>,
    pending: Arc<AtomicUsize>,
    preview: Arc<AtomicU64>,
}
struct RecoveryStore {
    root: PathBuf,
    session: Mutex<Option<Arc<storage::Recovery>>>,
}
impl RecoveryStore {
    fn acquire(&self) -> Result<Arc<storage::Recovery>, String> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Recovery initialization failed".to_owned())?;
        if let Some(value) = &*session {
            return Ok(value.clone());
        }
        let value = Arc::new(storage::Recovery::new(&self.root)?);
        *session = Some(value.clone());
        Ok(value)
    }
    fn cached(&self) -> Option<Arc<storage::Recovery>> {
        self.session
            .try_lock()
            .ok()
            .and_then(|session| session.clone())
    }
}
fn tracked(job: &Job) -> bool {
    #[cfg(test)]
    if matches!(job, Job::Hold(..) | Job::HoldMaintenance(..)) {
        return true;
    }
    matches!(
        job,
        Job::Create(..)
            | Job::Rename(..)
            | Job::Trash(..)
            | Job::TrashRecord(..)
            | Job::Save { .. }
            | Job::SaveTo(..)
            | Job::Settings(..)
    )
}
impl IoWorker {
    pub fn new(ctx: wake::Context) -> Self {
        Self::new_in(ctx, storage::data_dir().join("recovery"))
    }
    fn new_in(ctx: wake::Context, root: PathBuf) -> Self {
        let (jobs, writes) = mpsc::sync_channel(16);
        let (reads, read_jobs) = mpsc::sync_channel(32);
        let (maintenance, maintenance_jobs) = mpsc::sync_channel(16);
        let (results, replies) = mpsc::channel();
        let errors = results.clone();
        let drafts = Arc::new(Mutex::new(HashMap::<u64, storage::Draft>::new()));
        let preview = Arc::new(AtomicU64::new(0));
        let recovery_store = Arc::new(RecoveryStore {
            root: root.clone(),
            session: Mutex::new(None),
        });
        let clears = Arc::new(Mutex::new(HashMap::<u64, u64>::new()));
        let cleared_through = Arc::new(Mutex::new(HashMap::<u64, u64>::new()));
        let pending = Arc::new(AtomicUsize::new(0));
        let start = |requests: Arc<Mutex<mpsc::Receiver<Job>>>, maintenance: bool| {
            let (results, pending_drafts, store, ctx) = (
                results.clone(),
                drafts.clone(),
                recovery_store.clone(),
                ctx.clone(),
            );
            let pending_clears = clears.clone();
            let count = pending.clone();
            let root = root.clone();
            let preview = preview.clone();
            thread::spawn(move || {
                let rx = Requests(requests);
                let mut settings_base: Option<Option<Vec<u8>>> = None;
                while let Ok(job) = rx.recv() {
                    let updates = if maintenance {
                        std::mem::take(&mut *pending_drafts.lock().unwrap())
                    } else {
                        HashMap::new()
                    };
                    let removals = if maintenance {
                        std::mem::take(&mut *pending_clears.lock().unwrap())
                    } else {
                        HashMap::new()
                    };
                    let needed = !updates.is_empty()
                        || match &job {
                            Job::Import(..)
                            | Job::RestoreHistory(..)
                            | Job::RestoreObserved(..)
                            | Job::Recover(..) => true,
                            Job::Save {
                                without_recovery, ..
                            } => !*without_recovery,
                            Job::SaveTo(destination) => !destination.without_recovery,
                            _ => false,
                        };
                    let recovery = Arc::new(if needed {
                        store.acquire()
                    } else {
                        store
                            .cached()
                            .ok_or_else(|| "Recovery has not been initialized".into())
                    });
                    for (id, generation) in removals {
                        let result = match recovery.as_ref() {
                            Ok(r) => r.saved(id, generation),
                            Err(_) => Ok(()),
                        };
                        let _ = results.send(Reply::Cleared(id, generation, result));
                        ctx.request_repaint();
                    }
                    for (id, draft) in updates {
                        if let Err(error) = recovery
                            .as_ref()
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|r| r.write(id, &draft))
                        {
                            let _ =
                                results.send(Reply::Done(Err(format!("Recovery failed: {error}"))));
                            ctx.request_repaint();
                        }
                    }
                    let was_tracked = tracked(&job);
                    let observed_snapshot = matches!(&job, Job::RestoreObserved(..));
                    let reply = match job {
                        Job::RecentLocation(paths) => Reply::RecentLocation(
                            paths
                                .into_iter()
                                .find(|path| std::fs::read_dir(path).is_ok()),
                        ),
                        #[cfg(test)]
                        Job::Hold(started, release) | Job::HoldMaintenance(started, release) => {
                            let _ = started.send(());
                            let _ = release.recv();
                            if was_tracked {
                                count.fetch_sub(1, Ordering::AcqRel);
                            }
                            continue;
                        }
                        Job::Wake => continue,
                        Job::Inventory => Reply::Records(storage::inventory(&root)),
                        Job::RestoreHistory(id, path) | Job::RestoreObserved(id, path) => {
                            Reply::Imported(
                                id,
                                (|| {
                                    let draft =
                                        storage::restore_snapshot(&path, observed_snapshot)?;
                                    recovery
                                        .as_ref()
                                        .as_ref()
                                        .map_err(Clone::clone)?
                                        .write(id, &draft)?;
                                    let disk = DiskFile::restore(&draft);
                                    Ok((draft, disk))
                                })(),
                            )
                        }
                        Job::Import(id, source) => Reply::Imported(
                            id,
                            recovery
                                .as_ref()
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|r| r.claim(&source, id))
                                .map(|(draft, warning)| {
                                    if let Some(warning) = warning {
                                        let _ = results.send(Reply::HistoryWarning(warning));
                                    }
                                    let disk = DiskFile::restore(&draft);
                                    (draft, disk)
                                }),
                        ),
                        Job::External(path, reveal) => Reply::Done(
                            std::process::Command::new("/usr/bin/open")
                                .args(if reveal {
                                    vec![std::ffi::OsStr::new("-R"), path.as_os_str()]
                                } else {
                                    vec![path.as_os_str()]
                                })
                                .spawn()
                                .map(|_| ())
                                .map_err(|e| e.to_string()),
                        ),
                        Job::Visit(path) => {
                            let folder = path.is_dir();
                            Reply::Visit(path, folder)
                        }
                        Job::Choose(path, directory) => {
                            let result = std::fs::metadata(&path)
                                .map_err(|e| e.to_string())
                                .and_then(|m| {
                                    if (directory && m.is_dir()) || (!directory && m.is_file()) {
                                        Ok(())
                                    } else {
                                        Err(if directory {
                                            "Choose a folder"
                                        } else {
                                            "Choose a file"
                                        }
                                        .into())
                                    }
                                });
                            Reply::Chosen(path, result)
                        }
                        Job::Finish(id, generation, disk, paths) => {
                            Reply::Finished(id, generation, disk.revalidate_paths(&paths))
                        }
                        Job::Check(id, path, generation) => {
                            Reply::Checked(id, generation, DiskFile::open(path))
                        }
                        Job::Settings(settings) => {
                            let base =
                                settings_base.get_or_insert_with(|| settings.disk_base.clone());
                            Reply::Done(settings.save_checked(&storage::data_dir(), base))
                        }
                        Job::LoadSettings => {
                            let result = storage::Settings::load();
                            if let Ok(value) = &result {
                                settings_base = Some(value.disk_base.clone());
                            }
                            Reply::SettingsLoaded(result)
                        }
                        Job::Recover(id, draft) => Reply::Done(
                            recovery
                                .as_ref()
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|r| r.write(id, &draft)),
                        ),
                        Job::ClearThrough(id, generation) => Reply::Cleared(
                            id,
                            generation,
                            recovery
                                .as_ref()
                                .as_ref()
                                .map_or(Ok(()), |r| r.saved(id, generation)),
                        ),
                        Job::Create(path, folder, text) => {
                            Reply::Changed(fileops::create(&path, folder, &text))
                        }
                        Job::Rename(from, to) => Reply::Changed(fileops::rename(&from, &to)),
                        Job::TrashRecord(path) => {
                            Reply::RecordTrashed(storage::trash_record(&path))
                        }
                        Job::Trash(path) => Reply::Changed(fileops::trash(&path)),
                        Job::Prepare(destination) => {
                            let result = match std::fs::symlink_metadata(&destination.path) {
                                Ok(_) => {
                                    DiskFile::open(destination.path.clone()).map(|v| Some(v.0))
                                }
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                    Ok(None)
                                }
                                Err(error) => Err(error.to_string()),
                            };
                            Reply::Prepared(Box::new(destination), result)
                        }
                        Job::SaveTo(destination) => {
                            let result = save_destination(
                                &destination,
                                recovery
                                    .as_ref()
                                    .as_ref()
                                    .map(|r| &**r)
                                    .map_err(Clone::clone),
                                &results,
                            );
                            Reply::SavedTo(Box::new(destination), result)
                        }
                        Job::Open(path, token, transient)
                            if (!transient || preview.load(Ordering::Relaxed) == token)
                                && path.is_dir() =>
                        {
                            Reply::OpenDirectory(path, token)
                        }
                        Job::Open(path, token, transient) => Reply::Open(
                            path.clone(),
                            token,
                            DiskFile::open_cancellable(path, &|| {
                                transient && preview.load(Ordering::Relaxed) != token
                            }),
                        ),
                        Job::Save {
                            id,
                            disk,
                            text,
                            generation,
                            without_recovery,
                        } => {
                            let result = if without_recovery {
                                disk.save(&text)
                            } else {
                                recovery
                                    .as_ref()
                                    .as_ref()
                                    .map_err(|e| format!("Recovery unavailable: {e}"))
                                    .and_then(|r| {
                                        let candidate = r
                                            .checkpoint(
                                                id,
                                                &storage::Draft {
                                                    path: disk.path.clone(),
                                                    text: text.clone(),
                                                    base: disk.bytes.clone(),
                                                    target: Some(disk.target.clone()),
                                                    generation,
                                                },
                                            )
                                            .map_err(|e| format!("Recovery unavailable: {e}"))?;
                                        let saved = disk.save(&text)?;
                                        if let Err(error) = r.saved(id, generation) {
                                            let _ = results.send(Reply::HistoryWarning(error));
                                        }
                                        if let Err(error) = r.resolved(&candidate) {
                                            let _ = results.send(Reply::HistoryWarning(error));
                                        }
                                        Ok(saved)
                                    })
                            };
                            Reply::Save(id, text, result)
                        }
                    };
                    if was_tracked {
                        count.fetch_sub(1, Ordering::AcqRel);
                    }
                    if results.send(reply).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
        };
        let handles = vec![
            start(Arc::new(Mutex::new(writes)), false),
            start(Arc::new(Mutex::new(maintenance_jobs)), true),
        ];
        let read_jobs = Arc::new(Mutex::new(read_jobs));
        for _ in 0..2 {
            start(read_jobs.clone(), false);
        }
        Self {
            jobs: Some(jobs),
            reads: Some(reads),
            maintenance: Some(maintenance),
            replies,
            errors,
            drafts,
            handles,
            clears,
            cleared_through,
            pending,
            preview,
        }
    }
    pub fn pending_mutations(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
    pub fn cancel_previews(&self, token: u64) {
        self.preview.store(token, Ordering::Relaxed);
    }
    pub fn send(&self, job: Job) {
        if let Job::Open(_, token, _) = &job {
            self.preview.store(*token, Ordering::Relaxed);
        }
        let clearing = match &job {
            Job::ClearThrough(id, generation) => Some((*id, *generation)),
            _ => None,
        };
        if let Some((id, generation)) = clearing {
            self.cleared_through
                .lock()
                .unwrap()
                .entry(id)
                .and_modify(|g| *g = (*g).max(generation))
                .or_insert(generation);
            self.drafts
                .lock()
                .unwrap()
                .retain(|key, draft| *key != id || draft.generation > generation);
            self.clears
                .lock()
                .unwrap()
                .entry(id)
                .and_modify(|g| *g = (*g).max(generation))
                .or_insert(generation);
            if let Some(tx) = &self.maintenance
                && let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(Job::Wake)
            {
                let _ = self.errors.send(Reply::Cleared(
                    id,
                    generation,
                    Err("Recovery service stopped; record may remain".into()),
                ));
            }
            return;
        }
        let job = if let Job::Recover(id, draft) = job {
            if self
                .cleared_through
                .lock()
                .unwrap()
                .get(&id)
                .is_some_and(|g| *g >= draft.generation)
            {
                return;
            }
            self.drafts.lock().unwrap().insert(id, draft);
            if let Some(tx) = &self.maintenance {
                let _ = tx.try_send(Job::Wake);
            }
            return;
        } else {
            job
        };
        let channel = match &job {
            #[cfg(test)]
            Job::HoldMaintenance(..) => &self.maintenance,
            Job::Open(..)
            | Job::Choose(..)
            | Job::Finish(..)
            | Job::Check(..)
            | Job::Visit(_)
            | Job::RecentLocation(_)
            | Job::Prepare(_)
            | Job::Import(..)
            | Job::RestoreHistory(..)
            | Job::RestoreObserved(..) => &self.reads,
            Job::Wake
            | Job::ClearThrough(..)
            | Job::Settings(_)
            | Job::LoadSettings
            | Job::Inventory => &self.maintenance,
            _ => &self.jobs,
        };
        if let Some(tx) = channel {
            let counted = tracked(&job);
            if counted {
                self.pending.fetch_add(1, Ordering::AcqRel);
            }
            if let Err(error) = tx.try_send(job) {
                if counted {
                    self.pending.fetch_sub(1, Ordering::AcqRel);
                }
                let job = match error {
                    mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
                };
                let message = "I/O queue unavailable. Operation was not queued; retry. Current buffer remains in memory.".to_owned();
                let reply = match job {
                    Job::RecentLocation(_) => Reply::RecentLocation(None),
                    Job::Save { id, text, .. } => Reply::Save(id, text, Err(message)),
                    Job::SaveTo(d) | Job::Prepare(d) => Reply::SavedTo(Box::new(d), Err(message)),
                    Job::Choose(path, _) => Reply::Chosen(path, Err(message)),
                    Job::Finish(id, generation, _, _) => {
                        Reply::Finished(id, generation, Err(message))
                    }
                    Job::Open(path, token, _) => Reply::Open(path, token, Err(message)),
                    _ => Reply::Done(Err(message)),
                };
                let _ = self.errors.send(reply);
            }
        }
    }
}
fn save_destination(
    destination: &SaveDestination,
    recovery: Result<&storage::Recovery, String>,
    results: &mpsc::Sender<Reply>,
) -> Result<DiskFile, String> {
    if destination.copy
        && destination
            .expected
            .as_ref()
            .is_some_and(|disk| disk.same_file(&destination.original))
    {
        return Err("Save Copy requires a separate destination".into());
    }
    let candidate = if destination.without_recovery {
        None
    } else {
        let r = recovery
            .as_ref()
            .map_err(|e| format!("Recovery unavailable: {e}"))?;
        let draft = storage::Draft {
            path: destination.original.path.clone(),
            text: destination.text.clone(),
            base: destination.original.bytes.clone(),
            target: Some(destination.original.target.clone()),
            generation: destination.generation,
        };
        r.write(destination.id, &draft)
            .map_err(|e| format!("Recovery unavailable: {e}"))?;
        Some(
            r.checkpoint_version(&storage::Draft {
                path: destination.path.clone(),
                text: destination.text.clone(),
                base: destination
                    .expected
                    .as_ref()
                    .map(|d| d.bytes.clone())
                    .unwrap_or_default(),
                target: destination.expected.as_ref().map(|d| d.target.clone()),
                generation: destination.generation,
            })
            .map_err(|e| format!("Recovery unavailable: {e}"))?,
        )
    };
    let disk = if let Some(expected) = &destination.expected {
        expected.save_with_bom(&destination.text, destination.bom)?
    } else {
        fileops::create(
            &destination.path,
            false,
            &if destination.bom {
                format!("\u{feff}{}", destination.text)
            } else {
                destination.text.clone()
            },
        )?;
        DiskFile::open(destination.path.clone())?.0
    };
    if let Ok(r) = recovery {
        if !destination.copy
            && let Err(error) = r.saved(destination.id, destination.generation)
        {
            let _ = results.send(Reply::HistoryWarning(error));
        }
        if let Some(candidate) = candidate
            && let Err(error) = r.resolved(&candidate)
        {
            let _ = results.send(Reply::HistoryWarning(error));
        }
    }
    Ok(disk)
}
struct Requests(Arc<Mutex<mpsc::Receiver<Job>>>);
impl Requests {
    fn recv(&self) -> Result<Job, mpsc::RecvError> {
        self.0.lock().unwrap().recv()
    }
}
impl Drop for IoWorker {
    fn drop(&mut self) {
        self.jobs.take();
        self.reads.take();
        self.maintenance.take();
        self.handles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_recent_location_skips_missing_files_and_uses_native_paths() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder-中文-é");
        std::fs::create_dir(&folder).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, "").unwrap();
        let io = IoWorker::new_in(wake::Context::default(), root.path().join("recovery"));
        io.send(Job::RecentLocation(vec![
            root.path().join("missing"),
            file,
            folder.clone(),
        ]));
        match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::RecentLocation(path) => assert_eq!(path, Some(folder)),
            _ => panic!("unexpected reply"),
        }
        assert!(!root.path().join("recovery").exists());
    }
    fn destination(path: PathBuf, expected: Option<DiskFile>) -> SaveDestination {
        SaveDestination {
            id: 7,
            path,
            text: "mine".into(),
            copy: true,
            bom: false,
            generation: 1,
            expected,
            original: DiskFile::draft(),
            without_recovery: false,
        }
    }
    #[test]
    fn existing_destination_is_revalidated_and_recovery_override_is_narrow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dest");
        std::fs::write(&path, "base").unwrap();
        let expected = DiskFile::open(path.clone()).unwrap().0;
        let mut d = destination(path.clone(), Some(expected));
        let (tx, _) = mpsc::channel();
        assert!(
            save_destination(&d, Err("unavailable".into()), &tx)
                .err()
                .unwrap()
                .starts_with("Recovery unavailable:")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "base");
        d.without_recovery = true;
        std::fs::write(&path, "else").unwrap();
        assert!(save_destination(&d, Err("unavailable".into()), &tx).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "else");
        d.expected = Some(DiskFile::open(path.clone()).unwrap().0);
        save_destination(&d, Err("unavailable".into()), &tx).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "mine");
    }
    #[test]
    fn destination_race_never_clobbers_and_history_failure_does_not_undo_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dest");
        let r = storage::Recovery::new(&dir.path().join("recovery")).unwrap();
        let d = destination(path.clone(), None);
        let (tx, rx) = mpsc::channel();
        std::fs::write(&path, "racer").unwrap();
        assert!(save_destination(&d, Ok(&r), &tx).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "racer");
        let mut d = destination(path.clone(), Some(DiskFile::open(path.clone()).unwrap().0));
        d.copy = false;
        std::fs::write(dir.path().join("recovery/history"), "not a folder").unwrap();
        save_destination(&d, Ok(&r), &tx).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "mine");
        assert!(matches!(rx.recv().unwrap(), Reply::HistoryWarning(_)));
    }
    fn draft(generation: u64) -> storage::Draft {
        storage::Draft {
            path: "unbound".into(),
            text: "local".into(),
            base: vec![],
            target: None,
            generation,
        }
    }
    fn barrier(io: &IoWorker) {
        io.maintenance
            .as_ref()
            .unwrap()
            .send(Job::Inventory)
            .unwrap();
        loop {
            if matches!(
                io.replies
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap(),
                Reply::Records(_)
            ) {
                break;
            }
        }
    }
    fn record(root: &std::path::Path, id: u64) -> PathBuf {
        std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|entry| entry.path().join(format!("{id}.json")))
            .find(|path| path.exists())
            .expect("record written")
    }
    #[test]
    fn recovery_initialization_is_lazy_and_retryable() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recovery");
        let io = IoWorker::new_in(wake::Context::default(), path.clone());
        assert!(!path.exists());
        std::fs::write(&path, "blocked").unwrap();
        io.send(Job::Recover(7, draft(1)));
        loop {
            if let Reply::Done(Err(error)) = io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                assert!(error.contains("Recovery failed"));
                break;
            }
        }
        std::fs::remove_file(&path).unwrap();
        io.send(Job::Recover(7, draft(2)));
        barrier(&io);
        let restored: storage::Draft =
            serde_json::from_slice(&std::fs::read(record(&path, 7)).unwrap()).unwrap();
        assert_eq!(restored.generation, 2);
    }
    #[test]
    fn full_maintenance_queue_does_not_lose_clear_or_resurrect_an_old_generation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recovery");
        let io = IoWorker::new_in(wake::Context::default(), path.clone());
        let (started, seen) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        io.send(Job::HoldMaintenance(started, hold));
        seen.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        io.send(Job::Recover(3, draft(1)));
        while io.maintenance.as_ref().unwrap().try_send(Job::Wake).is_ok() {}
        io.send(Job::ClearThrough(3, 1));
        release.send(()).unwrap();
        loop {
            if let Reply::Cleared(3, 1, result) = io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                result.unwrap();
                break;
            }
        }
        io.send(Job::Recover(3, draft(1)));
        barrier(&io);
        assert!(!path.exists());
        io.send(Job::Recover(3, draft(2)));
        barrier(&io);
        let restored: storage::Draft =
            serde_json::from_slice(&std::fs::read(record(&path, 3)).unwrap()).unwrap();
        assert_eq!(restored.generation, 2);
    }
    #[test]
    fn cleanup_failure_is_acknowledged_and_can_be_retried() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recovery");
        let io = IoWorker::new_in(wake::Context::default(), path.clone());
        io.send(Job::Recover(3, draft(1)));
        barrier(&io);
        let path = record(&path, 3);
        let original = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("blocker"), "test").unwrap();
        io.send(Job::ClearThrough(3, 1));
        loop {
            if let Reply::Cleared(3, 1, result) = io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                assert!(result.is_err());
                break;
            }
        }
        std::fs::remove_file(path.join("blocker")).unwrap();
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, original).unwrap();
        io.send(Job::ClearThrough(3, 1));
        loop {
            if let Reply::Cleared(3, 1, result) = io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                result.unwrap();
                break;
            }
        }
        assert!(!path.exists());
    }
    #[test]
    fn frontend_drop_does_not_join_stalled_work() {
        let root = tempfile::tempdir().unwrap();
        let io = IoWorker::new_in(wake::Context::default(), root.path().join("recovery"));
        let (started, seen) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        io.send(Job::Hold(started, hold));
        seen.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(io.pending_mutations(), 1);
        let started = std::time::Instant::now();
        drop(io);
        let elapsed = started.elapsed();
        release.send(()).unwrap();
        assert!(elapsed < std::time::Duration::from_millis(100));
    }
    #[test]
    fn blocked_mutation_does_not_block_document_reads() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("read.txt");
        std::fs::write(&path, "available").unwrap();
        let io = IoWorker::new_in(wake::Context::default(), root.path().join("recovery"));
        let (started, seen) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        io.send(Job::Hold(started, hold));
        seen.recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        io.send(Job::Open(path, 1, false));
        let reply = io.replies.recv_timeout(std::time::Duration::from_secs(2));
        release.send(()).unwrap();
        match reply.unwrap() {
            Reply::Open(_, _, Ok((_, text))) => assert_eq!(text, "available"),
            _ => panic!("document worker did not run independently"),
        }
    }
}
