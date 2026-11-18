use crate::wake;
use crate::{
    document::{DiskFile, Observation},
    fileops, storage,
};
use gpui_component::{
    highlighter::HighlightTheme,
    input::{PreparedInput, Rope},
};
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
    #[cfg(test)]
    HoldRead(mpsc::Sender<()>, mpsc::Receiver<()>),
    ReadMarkdown {
        request: u64,
        source: Rope,
        path: PathBuf,
        theme: Arc<HighlightTheme>,
        layout: Box<gpui_component::text::MarkdownLayout>,
        reclaim: Option<fn()>,
    },
    CompletePath(u64, String, PathBuf),
    Visit(PathBuf),
    RecentLocation(Vec<PathBuf>),
    Volumes,
    External(PathBuf, bool),
    Create(PathBuf, bool, String),
    Rename(PathBuf, PathBuf),
    Trash(PathBuf),
    TrashRecord(PathBuf),
    Prepare(SaveDestination),
    SaveTo(SaveDestination),
    Reload(Box<Reload>),
    BindingApplied(u64, u64, u64),
    Recover(u64, storage::Draft),
    Wake,
    Inventory,
    RestoreHistory(u64, PathBuf),
    RestoreObserved(u64, PathBuf),
    Import(u64, PathBuf),
    ClearThrough(u64, u64),
    Settings(storage::Settings),
    LoadSettings,
    Check(u64, DiskFile, u64),
    Finish(u64, u64, u64, DiskFile, Vec<PathBuf>),
    Choose(PathBuf, bool),
    Open(PathBuf, u64, bool),
    Save {
        id: u64,
        disk: DiskFile,
        text: TextSnapshot,
        generation: u64,
        without_recovery: bool,
    },
}
pub struct Reload {
    pub id: u64,
    pub generation: u64,
    pub original: DiskFile,
    pub local: String,
    pub disk: DiskFile,
    pub text: Arc<str>,
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
    pub recovery_copy: bool,
}
pub enum Reply {
    Markdown(u64, Result<Box<crate::reading::Prepared>, String>),
    PathCompleted(u64, String, Result<crate::path_completion::Matches, String>),
    RecentLocation(Option<PathBuf>),
    Volumes(Result<crate::locations::Volumes, String>),
    SettingsLoaded(Result<storage::Settings, String>),
    SettingsSaved(Result<storage::Settings, String>),
    Changed(Result<(), String>),
    Records(Result<storage::Inventory, String>),
    Finished(u64, u64, u64, DiskFile, Result<(), String>),
    Chosen(PathBuf, Result<(), String>),
    OpenDirectory(PathBuf, u64),
    HistoryWarning(String),
    Cleared(u64, u64, Result<(), String>),
    RecordTrashed(Result<(), String>),
    Prepared(Box<SaveDestination>, Result<Option<DiskFile>, String>),
    Imported(u64, Result<(storage::Draft, DiskFile), String>),
    Reloaded(Box<Reload>, Result<(), String>),
    Recovered(u64, u64, Result<(), String>),
    Done(Result<(), String>),
    Visit(PathBuf, bool),
    SavedTo(Box<SaveDestination>, Result<DiskFile, String>),
    Checked(u64, u64, Result<Observation, String>),
    CheckDeferred(u64, u64),
    Open(PathBuf, u64, Result<(DiskFile, PreparedInput), String>),
    Save(u64, u64, Result<DiskFile, String>),
}

struct Rebinding {
    generation: u64,
    through: AtomicU64,
    from: DiskFile,
    to: DiskFile,
}
impl Rebinding {
    fn apply(&self, draft: &mut storage::Draft<impl Sized>) {
        if draft.generation > self.generation
            && draft.generation <= self.through.load(Ordering::Acquire)
            && draft.path == self.from.path
            && draft.target.as_deref().unwrap_or(std::path::Path::new("")) == self.from.target
        {
            draft.path = self.to.path.clone();
            draft.target = Some(self.to.target.clone());
            draft.base = self.to.bytes.clone();
        }
    }
}
#[derive(Default)]
struct Rebindings {
    entries: Mutex<HashMap<u64, Arc<Rebinding>>>,
    writes: Mutex<()>,
}
pub struct IoWorker {
    jobs: Option<mpsc::SyncSender<Job>>,
    reads: Option<mpsc::SyncSender<Job>>,
    maintenance: Option<mpsc::SyncSender<Job>>,
    pub replies: mpsc::Receiver<Reply>,
    errors: mpsc::Sender<Reply>,
    drafts: Arc<Mutex<HashMap<u64, storage::Draft<TextSnapshot>>>>,
    handles: Vec<thread::JoinHandle<()>>,
    clears: Arc<Mutex<HashMap<u64, u64>>>,
    cleared_through: Arc<Mutex<HashMap<u64, u64>>>,
    pending: Arc<AtomicUsize>,
    preview: Arc<AtomicU64>,
    reading: Arc<AtomicU64>,
    wake: wake::Context,
    rebindings: Arc<Rebindings>,
}

pub enum TextSnapshot {
    Flat(Arc<str>),
    Revision(Rope),
}

impl serde::Serialize for TextSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Flat(text) => serializer.serialize_str(text),
            Self::Revision(text) => serializer.collect_str(text),
        }
    }
}
impl storage::RecoveryText for TextSnapshot {
    fn byte_len(&self) -> usize {
        match self {
            Self::Flat(text) => text.len(),
            Self::Revision(text) => text.len(),
        }
    }
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
            | Job::Reload(..)
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
        let drafts = Arc::new(Mutex::new(
            HashMap::<u64, storage::Draft<TextSnapshot>>::new(),
        ));
        let preview = Arc::new(AtomicU64::new(0));
        let reading = Arc::new(AtomicU64::new(0));
        let rebindings = Arc::new(Rebindings::default());
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
            let rebindings = rebindings.clone();
            let count = pending.clone();
            let root = root.clone();
            let preview = preview.clone();
            let reading = reading.clone();
            let settings_root = root.parent().unwrap_or(&root).to_owned();
            thread::spawn(move || {
                let rx = Requests(requests);
                while let Ok(job) = rx.recv() {
                    #[cfg(feature = "test-support")]
                    if matches!(&job, Job::Open(..)) {
                        crate::metrics::delay("OPEN");
                    }
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
                            | Job::Recover(..)
                            | Job::Reload(..) => true,
                            Job::Save {
                                without_recovery, ..
                            } => !*without_recovery,
                            Job::SaveTo(destination) => {
                                !destination.without_recovery && !destination.recovery_copy
                            }
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
                        crate::metrics::event("clear-worker", id, generation);
                        #[cfg(feature = "test-support")]
                        crate::metrics::delay("CLEAR");
                        let result = match recovery.as_ref() {
                            Ok(r) => r.saved(id, generation),
                            Err(_) => Ok(()),
                        };
                        crate::metrics::event("clear-worker-done", id, generation);
                        let _ = results.send(Reply::Cleared(id, generation, result));
                        ctx.request_repaint();
                    }
                    for (id, mut draft) in updates {
                        crate::metrics::event("recovery-worker", id, draft.generation);
                        let _writes = rebindings.writes.lock().unwrap();
                        let binding = rebindings.entries.lock().unwrap().get(&id).cloned();
                        if let Some(binding) = binding {
                            binding.apply(&mut draft);
                        }
                        let result = recovery
                            .as_ref()
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|r| r.write(id, &draft))
                            .map_err(|error| format!("Recovery failed: {error}"));
                        crate::metrics::event("recovery-worker-done", id, draft.generation);
                        let _ = results.send(Reply::Recovered(id, draft.generation, result));
                        ctx.request_repaint();
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
                        Job::Hold(started, release)
                        | Job::HoldMaintenance(started, release)
                        | Job::HoldRead(started, release) => {
                            let _ = started.send(());
                            let _ = release.recv();
                            if was_tracked {
                                count.fetch_sub(1, Ordering::AcqRel);
                            }
                            continue;
                        }
                        Job::Wake => continue,
                        Job::BindingApplied(id, generation, _) => {
                            let mut bindings = rebindings.entries.lock().unwrap();
                            if bindings
                                .get(&id)
                                .is_some_and(|binding| binding.generation == generation)
                            {
                                bindings.remove(&id);
                            }
                            continue;
                        }
                        Job::Inventory => Reply::Records(storage::inventory(&root)),
                        Job::CompletePath(token, value, folder) => {
                            let result = crate::path_completion::complete(&value, &folder);
                            Reply::PathCompleted(token, value, result)
                        }
                        Job::RestoreHistory(id, path) | Job::RestoreObserved(id, path) => {
                            Reply::Imported(
                                id,
                                (|| {
                                    let mut draft =
                                        storage::restore_snapshot(&path, observed_snapshot)?;
                                    draft.target = None;
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
                                        let _ = results.send(Reply::HistoryWarning(format!(
                                            "Restored {}: {warning}",
                                            crate::paths::display(&draft.path)
                                        )));
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
                        Job::Finish(id, generation, attempt, disk, paths) => {
                            crate::metrics::event("finish-worker", id, generation);
                            #[cfg(feature = "test-support")]
                            crate::metrics::delay("FINISH");
                            let result = disk.revalidate_paths(&paths);
                            crate::metrics::event("finish-worker-done", id, generation);
                            Reply::Finished(id, generation, attempt, disk, result)
                        }
                        Job::ReadMarkdown {
                            request,
                            source,
                            path,
                            theme,
                            layout,
                            reclaim,
                        } => {
                            if reading.load(Ordering::Relaxed) != request {
                                continue;
                            }
                            #[cfg(feature = "test-support")]
                            {
                                crate::metrics::event("reading-request-held", request, 0);
                                crate::metrics::delay("READ_MARKDOWN");
                                if reading.load(Ordering::Relaxed) != request {
                                    crate::metrics::event("reading-request-cancelled", request, 0);
                                    continue;
                                }
                            }
                            let mut prepared =
                                crate::reading::prepare(String::from(source), &path, &theme);
                            if reading.load(Ordering::Relaxed) != request {
                                continue;
                            }
                            layout.prepare(&mut prepared.markdown);
                            if reading.load(Ordering::Relaxed) != request {
                                continue;
                            }
                            if let Some(reclaim) = reclaim {
                                let _work =
                                    gpui::work_trace::Span::new("reading_prepare_memory_release");
                                reclaim();
                            }
                            Reply::Markdown(request, Ok(Box::new(prepared)))
                        }
                        Job::Check(id, disk, generation) => {
                            crate::metrics::event("check-worker", id, generation);
                            #[cfg(feature = "test-support")]
                            crate::metrics::delay("CHECK");
                            let result = disk.observe();
                            crate::metrics::event("check-worker-done", id, generation);
                            Reply::Checked(id, generation, result)
                        }
                        Job::Volumes => Reply::Volumes(crate::locations::mounted()),
                        Job::Settings(mut settings) => {
                            let mut base = settings.disk_base.clone();
                            Reply::SettingsSaved(
                                settings.save_checked(&settings_root, &mut base).map(|()| {
                                    settings.disk_base = base.clone();
                                    settings
                                }),
                            )
                        }
                        Job::LoadSettings => {
                            Reply::SettingsLoaded(storage::Settings::load_in(&settings_root))
                        }
                        Job::Recover(id, draft) => Reply::Recovered(
                            id,
                            draft.generation,
                            recovery
                                .as_ref()
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|r| r.write(id, &draft))
                                .map_err(|error| format!("Recovery failed: {error}")),
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
                            crate::metrics::event(
                                "save-to-worker",
                                destination.id,
                                destination.generation,
                            );
                            #[cfg(feature = "test-support")]
                            crate::metrics::delay("SAVE_TO");
                            let mut result = save_destination(
                                &destination,
                                recovery
                                    .as_ref()
                                    .as_ref()
                                    .map(|r| &**r)
                                    .map_err(Clone::clone),
                                &results,
                            );
                            if !destination.copy {
                                result = result.and_then(|disk| {
                                    publish_binding(
                                        destination.id,
                                        destination.generation,
                                        &destination.original,
                                        disk,
                                        recovery
                                            .as_ref()
                                            .as_ref()
                                            .map(|r| &**r)
                                            .map_err(Clone::clone),
                                        &rebindings,
                                        destination.without_recovery,
                                    )
                                });
                            }
                            crate::metrics::event(
                                "save-to-worker-done",
                                destination.id,
                                destination.generation,
                            );
                            Reply::SavedTo(Box::new(destination), result)
                        }
                        Job::Open(path, token, transient)
                            if (!transient || preview.load(Ordering::Relaxed) == token)
                                && path.is_dir() =>
                        {
                            Reply::OpenDirectory(path, token)
                        }
                        Job::Open(path, token, transient) => {
                            let _work = gpui::work_trace::Span::new("open_document_worker");
                            let result = DiskFile::open_cancellable(path.clone(), &|| {
                                transient && preview.load(Ordering::Relaxed) != token
                            })
                            .map(|(disk, text)| {
                                let input = PreparedInput::new(text);
                                (disk.share_revision(input.text_revision().clone()), input)
                            });
                            Reply::Open(path, token, result)
                        }
                        Job::Reload(reload) => {
                            crate::metrics::event("reload-worker", reload.id, reload.generation);
                            #[cfg(feature = "test-support")]
                            crate::metrics::delay("RELOAD");
                            let result = reload_document(
                                &reload,
                                recovery
                                    .as_ref()
                                    .as_ref()
                                    .map(|r| &**r)
                                    .map_err(Clone::clone),
                                &results,
                            );
                            crate::metrics::event(
                                "reload-worker-done",
                                reload.id,
                                reload.generation,
                            );
                            Reply::Reloaded(reload, result)
                        }
                        Job::Save {
                            id,
                            disk,
                            text,
                            generation,
                            without_recovery,
                        } => {
                            crate::metrics::event("save-worker", id, generation);
                            #[cfg(feature = "test-support")]
                            crate::metrics::delay("SAVE");
                            let save = || match &text {
                                TextSnapshot::Flat(text) => disk.save_snapshot(text.clone()),
                                TextSnapshot::Revision(text) => disk.save_revision(text.clone()),
                            };
                            let result = if without_recovery {
                                save()
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
                                                    text: &text,
                                                    base: disk.bytes.clone(),
                                                    target: Some(disk.target.clone()),
                                                    generation,
                                                },
                                            )
                                            .map_err(|e| format!("Recovery unavailable: {e}"))?;
                                        let _ =
                                            results.send(Reply::Recovered(id, generation, Ok(())));
                                        let saved = save()?;
                                        if let Err(error) = r.saved(id, generation) {
                                            let _ = results.send(Reply::HistoryWarning(format!(
                                                "Saved {}. Recovery maintenance failed: {error}",
                                                crate::paths::display(&disk.path)
                                            )));
                                        }
                                        if let Err(error) = r.resolved(&candidate) {
                                            let _ = results.send(Reply::HistoryWarning(format!(
                                                "Saved {}. Recovery maintenance failed: {error}",
                                                crate::paths::display(&disk.path)
                                            )));
                                        }
                                        Ok(saved)
                                    })
                            };
                            let result = result.and_then(|saved| {
                                publish_binding(
                                    id,
                                    generation,
                                    &disk,
                                    saved,
                                    recovery
                                        .as_ref()
                                        .as_ref()
                                        .map(|r| &**r)
                                        .map_err(Clone::clone),
                                    &rebindings,
                                    without_recovery,
                                )
                            });
                            crate::metrics::event("save-worker-done", id, generation);
                            Reply::Save(id, generation, result)
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
            reading,
            wake: ctx,
            rebindings,
        }
    }
    pub fn pending_mutations(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
    pub fn cancel_previews(&self, token: u64) {
        self.preview.store(token, Ordering::Relaxed);
    }
    pub fn cancel_reading(&self) {
        self.reading.store(0, Ordering::Relaxed);
    }
    pub fn send(&self, job: Job) {
        if let Job::BindingApplied(id, saved, adopted) = &job
            && let Some(binding) = self
                .rebindings
                .entries
                .lock()
                .unwrap()
                .get(id)
                .filter(|binding| binding.generation == *saved)
        {
            binding
                .through
                .store(adopted.saturating_sub(1), Ordering::Release);
        }
        if let Job::Open(_, token, _) = &job {
            self.preview.store(*token, Ordering::Relaxed);
        }
        if let Job::ReadMarkdown { request, .. } = &job {
            self.reading.store(*request, Ordering::Relaxed);
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
                self.wake.request_repaint();
            }
            return;
        }
        let job = if let Job::Recover(id, draft) = job {
            self.queue_recovery(id, draft.map_text(TextSnapshot::Flat));
            return;
        } else {
            job
        };
        let channel = match &job {
            #[cfg(test)]
            Job::HoldRead(..) => &self.reads,
            #[cfg(test)]
            Job::HoldMaintenance(..) => &self.maintenance,
            Job::Volumes
            | Job::ReadMarkdown { .. }
            | Job::CompletePath(..)
            | Job::Open(..)
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
            | Job::Inventory
            | Job::BindingApplied(..) => &self.maintenance,
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
                let full = matches!(&error, mpsc::TrySendError::Full(_));
                let job = match error {
                    mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => job,
                };
                let message = "I/O queue unavailable. Operation was not queued; retry. Current buffer remains in memory.".to_owned();
                let reply = match job {
                    Job::Check(id, _, token) if full => Reply::CheckDeferred(id, token),
                    Job::Check(id, _, token) => Reply::Checked(id, token, Err(message)),
                    Job::ReadMarkdown { request, .. } => Reply::Markdown(request, Err(message)),
                    Job::CompletePath(token, value, _) => {
                        Reply::PathCompleted(token, value, Err(message))
                    }
                    Job::RecentLocation(_) => Reply::RecentLocation(None),
                    Job::Volumes => Reply::Volumes(Err(message)),
                    Job::Settings(_) => Reply::SettingsSaved(Err(message)),
                    Job::Save { id, generation, .. } => Reply::Save(id, generation, Err(message)),
                    Job::SaveTo(d) | Job::Prepare(d) => Reply::SavedTo(Box::new(d), Err(message)),
                    Job::Choose(path, _) => Reply::Chosen(path, Err(message)),
                    Job::Finish(id, generation, attempt, disk, _) => {
                        Reply::Finished(id, generation, attempt, disk, Err(message))
                    }
                    Job::Open(path, token, _) => Reply::Open(path, token, Err(message)),
                    Job::Import(id, _)
                    | Job::RestoreHistory(id, _)
                    | Job::RestoreObserved(id, _) => Reply::Imported(id, Err(message)),
                    Job::TrashRecord(_) => Reply::RecordTrashed(Err(message)),
                    Job::Reload(reload) => Reply::Reloaded(reload, Err(message)),
                    Job::BindingApplied(..) => Reply::Done(Ok(())),
                    _ => Reply::Done(Err(message)),
                };
                let _ = self.errors.send(reply);
                self.wake.request_repaint();
            }
        }
    }

    pub fn recover_edit(&self, id: u64, draft: storage::Draft<Rope>) {
        self.queue_recovery(id, draft.map_text(TextSnapshot::Revision));
    }

    fn queue_recovery(&self, id: u64, draft: storage::Draft<TextSnapshot>) {
        if self
            .cleared_through
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|generation| *generation >= draft.generation)
        {
            return;
        }
        let generation = draft.generation;
        self.drafts.lock().unwrap().insert(id, draft);
        if let Some(tx) = &self.maintenance
            && let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(Job::Wake)
        {
            let _ = self.errors.send(Reply::Recovered(
                id,
                generation,
                Err("Recovery service stopped; local text remains in memory".into()),
            ));
            self.wake.request_repaint();
        }
    }
}
fn publish_binding(
    id: u64,
    generation: u64,
    original: &DiskFile,
    disk: DiskFile,
    recovery: Result<&storage::Recovery, String>,
    rebindings: &Rebindings,
    without_recovery: bool,
) -> Result<DiskFile, String> {
    let _writes = rebindings.writes.lock().unwrap();
    if !without_recovery {
        recovery.and_then(|recovery| recovery.rebind(id, generation, original, &disk)).map_err(|error| format!("File was written at {}, but newer recovery could not be updated: {error}. The document is still associated with its previous file version.", crate::paths::display(&disk.path)))?;
    }
    let binding = Arc::new(Rebinding {
        generation,
        through: AtomicU64::new(u64::MAX),
        from: original.clone(),
        to: disk.clone(),
    });
    rebindings.entries.lock().unwrap().insert(id, binding);
    Ok(disk)
}

fn reload_document(
    reload: &Reload,
    recovery: Result<&storage::Recovery, String>,
    results: &mpsc::Sender<Reply>,
) -> Result<(), String> {
    reload
        .disk
        .revalidate_paths(std::slice::from_ref(&reload.disk.path))?;
    let recovery = recovery.map_err(|error| format!("Recovery unavailable: {error}"))?;
    let candidate = recovery
        .checkpoint(
            reload.id,
            &storage::Draft {
                path: reload.original.path.clone(),
                target: Some(reload.original.target.clone()),
                text: reload.local.as_str(),
                base: reload.original.bytes.clone(),
                generation: reload.generation,
            },
        )
        .map_err(|error| format!("Recovery unavailable: {error}"))?;
    reload
        .disk
        .revalidate_paths(std::slice::from_ref(&reload.disk.path))?;
    recovery
        .saved(reload.id, reload.generation)
        .map_err(|error| format!("Recovery unavailable: {error}"))?;
    if let Err(error) = recovery.resolved(&candidate) {
        let _ = results.send(Reply::HistoryWarning(format!(
            "Reloaded {}. Recovery history maintenance failed: {error}",
            crate::paths::display(&reload.disk.path)
        )));
    }
    Ok(())
}

fn save_destination(
    destination: &SaveDestination,
    recovery: Result<&storage::Recovery, String>,
    results: &mpsc::Sender<Reply>,
) -> Result<DiskFile, String> {
    if destination.recovery_copy {
        if !destination.copy || destination.expected.is_some() || destination.without_recovery {
            return Err(
                "A recovery copy requires a new filename and cannot replace an existing file"
                    .into(),
            );
        }
        return DiskFile::create(destination.path.clone(), &destination.text, destination.bom);
    }
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
        let draft: storage::Draft = storage::Draft {
            path: destination.original.path.clone(),
            text: destination.text.clone().into(),
            base: destination.original.bytes.clone(),
            target: Some(destination.original.target.clone()),
            generation: destination.generation,
        };
        r.write(destination.id, &draft)
            .map_err(|e| format!("Recovery unavailable: {e}"))?;
        Some(
            r.checkpoint_version(
                destination.id,
                &storage::Draft {
                    path: destination.path.clone(),
                    text: destination.text.as_str(),
                    base: destination
                        .expected
                        .as_ref()
                        .map(|d| d.bytes.clone())
                        .unwrap_or_default(),
                    target: destination.expected.as_ref().map(|d| d.target.clone()),
                    generation: destination.generation,
                },
            )
            .map_err(|e| format!("Recovery unavailable: {e}"))?,
        )
    };
    if candidate.is_some() {
        let _ = results.send(Reply::Recovered(
            destination.id,
            destination.generation,
            Ok(()),
        ));
    }
    let disk = if let Some(expected) = &destination.expected {
        expected.save_with_bom(&destination.text, destination.bom)?
    } else {
        DiskFile::create(destination.path.clone(), &destination.text, destination.bom)?
    };
    if let Ok(r) = recovery {
        if !destination.copy
            && let Err(error) = r.saved(destination.id, destination.generation)
        {
            let _ = results.send(Reply::HistoryWarning(format!(
                "Saved {}. Recovery maintenance failed: {error}",
                crate::paths::display(&destination.path)
            )));
        }
        if let Some(candidate) = candidate
            && let Err(error) = r.resolved(&candidate)
        {
            let _ = results.send(Reply::HistoryWarning(format!(
                "Saved {}. Recovery maintenance failed: {error}",
                crate::paths::display(&destination.path)
            )));
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
    fn shared_editor_recovery_preserves_the_wire_text() {
        let text = "\u{feff}café 👩‍👩‍👧‍👦 \"quoted\" \\ path\r\nline\rthird\n".repeat(450);
        let revision = Rope::from(text.as_str());
        assert!(revision.chunks().count() > 1);
        assert_eq!(
            serde_json::to_vec(&TextSnapshot::Revision(revision)).unwrap(),
            serde_json::to_vec(&text).unwrap()
        );
    }
    #[test]
    fn recovery_copies_are_new_files_and_never_replace_or_rebind() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("copy.txt");
        let (results, _) = mpsc::channel();
        let mut candidate = destination(path.clone(), None);
        candidate.text = "local β\r\n".into();
        candidate.bom = true;
        assert!(save_destination(&candidate, Err("storage unavailable".into()), &results).is_err());
        assert!(!path.exists());
        candidate.recovery_copy = true;
        let saved =
            save_destination(&candidate, Err("storage unavailable".into()), &results).unwrap();
        assert_eq!(
            saved.bytes.as_bytes().as_ref(),
            "\u{feff}local β\r\n".as_bytes()
        );
        candidate.expected = Some(saved.clone());
        assert!(save_destination(&candidate, Err("storage unavailable".into()), &results).is_err());
        candidate.expected = None;
        candidate.text = "replacement".into();
        assert!(save_destination(&candidate, Err("storage unavailable".into()), &results).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap().as_slice(),
            saved.bytes.as_bytes().as_ref()
        );
        candidate.path = root.path().join("not-created.txt");
        candidate.copy = false;
        assert!(save_destination(&candidate, Err("storage unavailable".into()), &results).is_err());
        assert!(!candidate.path.exists());
    }

    #[test]
    fn a_disconnected_queue_wakes_the_shell_with_a_typed_failure() {
        let root = tempfile::tempdir().unwrap();
        let (wake_tx, wake_rx) = mpsc::channel();
        let mut io = IoWorker::new_in(
            wake::Context::new(move || {
                let _ = wake_tx.send(());
            }),
            root.path().join("recovery"),
        );
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        io.jobs = Some(sender);
        let target = root.path().join("not-created.txt");
        io.send(Job::SaveTo(destination(target.clone(), None)));
        wake_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            io.replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            Reply::SavedTo(_, Err(_))
        ));
        assert!(!target.exists());
        assert_eq!(io.pending_mutations(), 0);
    }

    #[test]
    fn save_recovery_uses_the_completed_base_without_rewriting_later_observations() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original.txt");
        std::fs::write(&original, "original bytes\n").unwrap();
        let disk = DiskFile::open(original.clone()).unwrap().0;
        let recovery_root = root.path().join("recovery");
        let io = IoWorker::new_in(wake::Context::default(), recovery_root.clone());
        let draft = |generation, text: &str| storage::Draft {
            path: original.clone(),
            target: Some(disk.target.clone()),
            text: text.into(),
            base: disk.bytes.clone(),
            generation,
        };
        io.send(Job::Recover(7, draft(9, "newer local λ\n")));
        barrier(&io);
        io.send(Job::Save {
            id: 7,
            disk: disk.clone(),
            text: TextSnapshot::Revision(Rope::from("saved snapshot\n")),
            generation: 8,
            without_recovery: false,
        });
        loop {
            match io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                Reply::Save(7, 8, Ok(_)) => break,
                Reply::Save(_, _, Err(error)) => panic!("{error}"),
                _ => {}
            }
        }
        let path = record(&recovery_root, 7);
        let read =
            || serde_json::from_slice::<storage::Draft>(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read().base.as_bytes().as_ref(), b"saved snapshot\n");
        assert_eq!(read().text.as_ref(), "newer local λ\n");
        let (started, paused) = mpsc::channel();
        let (release, hold) = mpsc::channel();
        io.send(Job::HoldMaintenance(started, hold));
        paused
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        io.send(Job::Recover(7, draft(10, "late old-base edit\n")));
        io.send(Job::BindingApplied(7, 8, 12));
        io.send(Job::Recover(7, draft(12, "edit after a later reload\n")));
        release.send(()).unwrap();
        barrier(&io);
        assert_eq!(read().base.as_bytes().as_ref(), b"original bytes\n");
        assert_eq!(read().text.as_ref(), "edit after a later reload\n");
        assert_eq!(std::fs::read(&original).unwrap(), b"saved snapshot\n");
    }

    #[test]
    fn save_as_moves_newer_recovery_and_late_updates_to_the_new_binding() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original.txt");
        let target = root.path().join("new.txt");
        std::fs::write(&original, "original bytes\n").unwrap();
        let disk = DiskFile::open(original.clone()).unwrap().0;
        let recovery_root = root.path().join("recovery");
        let io = IoWorker::new_in(wake::Context::default(), recovery_root.clone());
        let draft = |generation, text: &str| storage::Draft {
            path: original.clone(),
            target: Some(disk.target.clone()),
            text: text.into(),
            base: disk.bytes.clone(),
            generation,
        };
        io.send(Job::Recover(7, draft(9, "newer local λ\n")));
        barrier(&io);
        io.send(Job::SaveTo(SaveDestination {
            id: 7,
            path: target.clone(),
            text: "saved snapshot\n".into(),
            copy: false,
            bom: false,
            generation: 8,
            expected: None,
            original: disk.clone(),
            without_recovery: false,
            recovery_copy: false,
        }));
        loop {
            match io
                .replies
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            {
                Reply::SavedTo(_, Ok(_)) => break,
                Reply::SavedTo(_, Err(error)) => panic!("{error}"),
                _ => {}
            }
        }
        let path = record(&recovery_root, 7);
        let read =
            || serde_json::from_slice::<storage::Draft>(&std::fs::read(&path).unwrap()).unwrap();
        let restored = read();
        assert_eq!(restored.path, target);
        assert_eq!(restored.target, Some(target.clone()));
        assert_eq!(restored.text.as_ref(), "newer local λ\n");
        assert_eq!(restored.base.as_bytes().as_ref(), b"saved snapshot\n");
        io.send(Job::Recover(7, draft(10, "late old-binding edit β\n")));
        io.send(Job::BindingApplied(7, 8, 11));
        barrier(&io);
        let restored = read();
        assert_eq!(restored.path, target);
        assert_eq!(restored.text.as_ref(), "late old-binding edit β\n");
        assert_eq!(restored.base.as_bytes().as_ref(), b"saved snapshot\n");
        io.send(Job::Recover(
            7,
            storage::Draft {
                path: target.clone(),
                target: Some(target.clone()),
                text: "new binding edit\n".into(),
                base: b"fresh observed base\n".as_slice().into(),
                generation: 11,
            },
        ));
        barrier(&io);
        assert_eq!(read().base.as_bytes().as_ref(), b"fresh observed base\n");
        assert_eq!(std::fs::read(&original).unwrap(), b"original bytes\n");
        assert_eq!(std::fs::read(&target).unwrap(), b"saved snapshot\n");
    }

    #[test]
    fn reload_validates_the_observed_version_and_retires_only_its_generation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("original.txt");
        std::fs::write(&path, "base\n").unwrap();
        let original = DiskFile::open(path.clone()).unwrap().0;
        let recovery = storage::Recovery::new(&root.path().join("recovery")).unwrap();
        std::fs::write(&path, "observed β\n").unwrap();
        let (disk, text) = DiskFile::open(path.clone()).unwrap();
        let reload = Reload {
            id: 4,
            generation: 8,
            original,
            local: "local λ\n".into(),
            disk,
            text,
        };
        let draft = |generation, text: &str| storage::Draft::<Arc<str>> {
            path: path.clone(),
            text: text.into(),
            base: reload.original.bytes.clone(),
            target: Some(path.clone()),
            generation,
        };
        recovery.write(4, &draft(8, &reload.local)).unwrap();
        let record = recovery.directory.join("4.json");
        let (results, _) = mpsc::channel();
        std::fs::write(&path, "later version\n").unwrap();
        assert!(reload_document(&reload, Ok(&recovery), &results).is_err());
        assert_eq!(
            serde_json::from_slice::<storage::Draft>(&std::fs::read(&record).unwrap())
                .unwrap()
                .text
                .as_ref(),
            reload.local.as_str()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "later version\n");
        std::fs::write(&path, reload.disk.bytes.as_bytes()).unwrap();
        recovery.write(4, &draft(9, "newer local edit\n")).unwrap();
        reload_document(&reload, Ok(&recovery), &results).unwrap();
        let retained: storage::Draft =
            serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        assert_eq!(retained.generation, 9);
        assert_eq!(retained.text.as_ref(), "newer local edit\n");
        assert_eq!(
            std::fs::read(&path).unwrap().as_slice(),
            reload.disk.bytes.as_bytes().as_ref()
        );
        assert!(
            storage::inventory(&root.path().join("recovery"))
                .unwrap()
                .records
                .iter()
                .any(|record| record.history)
        );
    }

    #[test]
    fn restored_history_is_an_unbound_copy_and_import_keeps_its_binding() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original.txt");
        std::fs::write(&original, b"\xef\xbb\xbfdisk\r\n").unwrap();
        let recovery_root = root.path().join("recovery");
        let source = storage::Recovery::new(&recovery_root).unwrap();
        let draft = storage::Draft {
            path: original.clone(),
            target: Some(original.clone()),
            text: "local β\r\n".into(),
            base: std::fs::read(&original).unwrap().into(),
            generation: 7,
        };
        source.write(41, &draft).unwrap();
        let record = source.directory.join("41.json");
        let snapshot = source.checkpoint_version(41, &draft).unwrap();
        source.resolved(&snapshot).unwrap();
        drop(source);
        let history = storage::inventory(&recovery_root)
            .unwrap()
            .records
            .into_iter()
            .find(|record| record.history)
            .unwrap()
            .path;
        let io = IoWorker::new_in(wake::Context::default(), recovery_root);
        io.send(Job::RestoreHistory(8, history.clone()));
        match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::Imported(8, Ok((restored, disk))) => {
                assert_eq!(restored.text, draft.text);
                assert!(disk.target.as_os_str().is_empty());
                assert!(disk.bom);
                assert!(history.exists());
            }
            _ => panic!("history restore failed"),
        }
        io.send(Job::Import(9, record.clone()));
        match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::Imported(9, Ok((restored, disk))) => {
                assert_eq!(restored.text, draft.text);
                assert_eq!(disk.target, original);
                assert_eq!(disk.bytes, draft.base);
                assert!(!record.exists());
            }
            _ => panic!("draft import failed"),
        }
        assert_eq!(
            std::fs::read(&original).unwrap().as_slice(),
            draft.base.as_bytes().as_ref()
        );
    }

    #[test]
    fn reloading_settings_cannot_authorize_a_stale_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let io = IoWorker::new_in(wake::Context::default(), root.path().join("recovery"));
        io.send(Job::Settings(storage::Settings::default()));
        let mut saved = match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::SettingsSaved(Ok(settings)) => settings,
            _ => panic!("initial settings write failed"),
        };
        let mut outside = saved.clone();
        outside.font_size = 22.;
        let external = serde_json::to_vec_pretty(&outside).unwrap();
        std::fs::write(root.path().join("settings.json"), &external).unwrap();
        io.send(Job::LoadSettings);
        match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::SettingsLoaded(Ok(settings)) => assert_eq!(settings.font_size, 22.),
            _ => panic!("settings reload failed"),
        }
        saved.font_size = 18.;
        io.send(Job::Settings(saved));
        match io
            .replies
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        {
            Reply::SettingsSaved(Err(error)) => assert!(error.contains("changed outside")),
            _ => panic!("stale settings snapshot replaced an external edit"),
        }
        assert_eq!(
            std::fs::read(root.path().join("settings.json")).unwrap(),
            external
        );
    }

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
            recovery_copy: false,
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
        assert!(matches!(rx.recv().unwrap(), Reply::Recovered(_, _, Ok(()))));
        let mut d = destination(path.clone(), Some(DiskFile::open(path.clone()).unwrap().0));
        d.copy = false;
        std::fs::write(dir.path().join("recovery/history"), "not a folder").unwrap();
        save_destination(&d, Ok(&r), &tx).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "mine");
        assert!(matches!(rx.recv().unwrap(), Reply::Recovered(_, _, Ok(()))));
        assert!(matches!(rx.recv().unwrap(), Reply::HistoryWarning(_)));
    }
    fn draft(generation: u64) -> storage::Draft {
        storage::Draft {
            path: "unbound".into(),
            text: "local".into(),
            base: Default::default(),
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
            if let Reply::Recovered(7, 1, Err(error)) = io
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
            Reply::Open(_, _, Ok((_, text))) => assert_eq!(text.value(), "available"),
            _ => panic!("document worker did not run independently"),
        }
    }
    #[test]
    fn full_read_queue_returns_a_typed_deferred_check_without_losing_queued_reads() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.txt");
        std::fs::write(&path, "original bytes").unwrap();
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        std::fs::write(&path, "disk bytes").unwrap();
        let io = IoWorker::new_in(wake::Context::default(), root.path().join("recovery"));
        let (entered, seen) = mpsc::channel();
        let mut holds = Vec::new();
        for _ in 0..2 {
            let (release, wait) = mpsc::channel();
            io.send(Job::HoldRead(entered.clone(), wait));
            holds.push(release);
        }
        for _ in 0..2 {
            seen.recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
        }
        for id in 0..32 {
            io.send(Job::Check(id, disk.clone(), id));
        }
        io.send(Job::Check(99, disk, 500));
        assert!(matches!(
            io.replies
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            Reply::CheckDeferred(99, 500)
        ));
        drop(holds);
        let mut completed = std::collections::HashSet::new();
        for _ in 0..32 {
            let Reply::Checked(id, token, result) = io
                .replies
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
            else {
                panic!("Expected checked file");
            };
            assert_eq!(id, token);
            let Observation::Changed(_, text) = result.unwrap() else {
                panic!("Expected changed bytes");
            };
            assert_eq!(text.as_ref(), "disk bytes");
            completed.insert(id);
        }
        assert_eq!(completed.len(), 32);
    }
}
