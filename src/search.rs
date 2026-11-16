use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub excerpt: String,
    pub query: String,
    pub fingerprint: Option<u64>,
    pub score: i64,
}
pub struct ResultSet {
    pub generation: u64,
    pub hits: Vec<Hit>,
    pub state: String,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Filters {
    pub ignored: bool,
    pub metadata: bool,
    pub exclude: String,
    pub file_bytes: usize,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            ignored: false,
            metadata: false,
            exclude: String::new(),
            file_bytes: 1024 * 1024,
        }
    }
}
type Request = (u64, PathBuf, String, bool, Filters);
struct FilenameIndex {
    scope: PathBuf,
    filters: Filters,
    paths: Arc<Vec<PathBuf>>,
    partial: bool,
    stale: Arc<AtomicBool>,
    _watcher: Option<notify::RecommendedWatcher>,
}
pub struct Search {
    index: Arc<Mutex<Option<FilenameIndex>>>,
    generation: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Request>>>,
    jobs: mpsc::SyncSender<()>,
    pub replies: mpsc::Receiver<ResultSet>,
}
pub fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    hash.finish()
}
fn rank(path: &Path, scope: &Path, query: &str) -> Option<i64> {
    let value = path
        .strip_prefix(scope)
        .unwrap_or(path)
        .to_string_lossy()
        .to_lowercase();
    let query = query.to_lowercase();
    let basename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if basename == query {
        return Some(10000);
    }
    if basename.starts_with(&query) {
        return Some(8000 - basename.len() as i64);
    }
    if let Some(index) = basename.find(&query) {
        return Some(6000 - index as i64);
    }
    if let Some(index) = value.find(&query) {
        return Some(4000 - index as i64);
    }
    let mut chars = query.chars();
    let mut next = chars.next();
    let mut gaps = 0;
    for c in value.chars() {
        if Some(c) == next {
            next = chars.next();
            if next.is_none() {
                return Some(1000 - gaps);
            }
        } else {
            gaps += 1;
        }
    }
    None
}
fn ordered(hits: &[Hit]) -> Vec<Hit> {
    let mut hits = hits.to_vec();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    hits
}
impl Search {
    pub fn new(ctx: crate::wake::Context) -> Self {
        let index = Arc::new(Mutex::new(None::<FilenameIndex>));
        let worker_index = index.clone();
        let generation = Arc::new(AtomicU64::new(0));
        let cancel = generation.clone();
        let latest = Arc::new(Mutex::new(None::<Request>));
        let pending = latest.clone();
        let (jobs, requests) = mpsc::sync_channel(1);
        let (results, replies) = mpsc::sync_channel(8);
        thread::spawn(move || {
            while requests.recv().is_ok() {
                let Some((id, scope, query, content, filters)) = pending.lock().unwrap().take()
                else {
                    continue;
                };
                let cancelled = || cancel.load(Ordering::Relaxed) != id;
                let send = |hits: &[Hit], state: &str| {
                    let mut result = ResultSet {
                        generation: id,
                        hits: ordered(hits),
                        state: state.into(),
                    };
                    loop {
                        if cancelled() {
                            return false;
                        }
                        match results.try_send(result) {
                            Ok(()) => {
                                ctx.request_repaint();
                                return true;
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => return false,
                            Err(mpsc::TrySendError::Full(value)) => {
                                result = value;
                                thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                };
                let building = query.is_empty() && !content;
                if !content && !building {
                    let cached = worker_index
                        .lock()
                        .unwrap()
                        .as_ref()
                        .filter(|index| index.scope == scope && index.filters == filters)
                        .map(|index| (index.paths.clone(), index.partial, index.stale.clone()));
                    if let Some((paths, partial, stale)) = cached {
                        let mut hits = Vec::new();
                        for path in paths.iter() {
                            if cancelled() {
                                break;
                            }
                            if let Some(score) = rank(path, &scope, &query) {
                                hits.push(Hit {
                                    path: path.clone(),
                                    line: 1,
                                    excerpt: String::new(),
                                    query: query.clone(),
                                    fingerprint: None,
                                    score,
                                });
                            }
                        }
                        hits = ordered(&hits);
                        let truncated = hits.len() > 1000;
                        hits.truncate(1000);
                        send(
                            &hits,
                            if stale.load(Ordering::Relaxed) {
                                "Indexed snapshot stale · rebuild for current results"
                            } else if partial || truncated {
                                "Indexed snapshot · partial results · rebuild for changes"
                            } else {
                                "Indexed snapshot · rebuild for changes"
                            },
                        );
                        continue;
                    }
                }
                let stale = Arc::new(AtomicBool::new(false));
                let mut index_watcher = None;
                if building {
                    use notify::Watcher;
                    let dirty = stale.clone();
                    let repaint = ctx.clone();
                    if let Ok(mut watcher) =
                        notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                            if event.as_ref().is_ok_and(|event| {
                                matches!(
                                    event.kind,
                                    notify::EventKind::Access(_) | notify::EventKind::Other
                                )
                            }) {
                                return;
                            }
                            if !dirty.swap(true, Ordering::Relaxed) {
                                repaint.request_repaint();
                            }
                        })
                        && watcher
                            .watch(&scope, notify::RecursiveMode::Recursive)
                            .is_ok()
                    {
                        index_watcher = Some(watcher);
                    }
                    if index_watcher.is_none() {
                        stale.store(true, Ordering::Relaxed);
                    }
                }
                let mut indexed_paths = Vec::new();
                let mut index_bytes = 0usize;
                let mut hits = Vec::new();
                let mut partial = false;
                let mut count = 0;
                let mut last = Instant::now();
                if !scope.is_dir() {
                    send(&[], "Failed — scope unavailable");
                    continue;
                }
                let mut exclusions = ignore::overrides::OverrideBuilder::new(&scope);
                let invalid = filters
                    .exclude
                    .lines()
                    .filter(|s| !s.trim().is_empty())
                    .find_map(|line| exclusions.add(&format!("!{line}")).err());
                if let Some(error) = invalid {
                    send(&[], &format!("Failed — invalid exclusion: {error}"));
                    continue;
                }
                let exclusions = match exclusions.build() {
                    Ok(value) => value,
                    Err(error) => {
                        send(&[], &format!("Failed — {error}"));
                        continue;
                    }
                };
                let metadata = filters.metadata;
                let file_bytes = filters.file_bytes.clamp(1, 10 * 1024 * 1024);
                let walker = ignore::WalkBuilder::new(&scope)
                    .hidden(false)
                    .git_ignore(!filters.ignored)
                    .ignore(!filters.ignored)
                    .git_exclude(!filters.ignored)
                    .require_git(false)
                    .follow_links(false)
                    .same_file_system(true)
                    .overrides(exclusions)
                    .filter_entry(move |entry| {
                        metadata
                            || ![".git", ".hg", ".svn"]
                                .iter()
                                .any(|name| entry.file_name() == *name)
                    })
                    .build();
                for entry in walker {
                    if cancelled() {
                        break;
                    }
                    count += 1;
                    if count > 100_000 || hits.len() >= 1000 {
                        partial = true;
                        break;
                    }
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(_) => {
                            partial = true;
                            continue;
                        }
                    };
                    if !entry.file_type().is_some_and(|t| t.is_file()) {
                        continue;
                    }
                    if building {
                        index_bytes += entry.path().as_os_str().len();
                        if index_bytes > 16 * 1024 * 1024 {
                            partial = true;
                            break;
                        }
                        indexed_paths.push(entry.path().to_path_buf());
                    } else if content {
                        let text = (|| -> std::io::Result<Option<Vec<u8>>> {
                            let mut bytes = Vec::new();
                            let mut file = std::fs::OpenOptions::new()
                                .read(true)
                                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
                                .open(entry.path())?;
                            if !file.metadata()?.is_file() {
                                return Err(std::io::Error::other("not a regular file"));
                            }
                            let mut chunk = [0u8; 65536];
                            loop {
                                if cancelled() {
                                    return Err(std::io::Error::other("cancelled"));
                                }
                                let read = file.read(&mut chunk)?;
                                if read == 0 {
                                    break;
                                }
                                if chunk[..read].iter().any(|b| {
                                    (*b < 32 && !matches!(*b, b'\t' | b'\r' | b'\n')) || *b == 127
                                }) {
                                    return Ok(None);
                                }
                                bytes.extend_from_slice(&chunk[..read]);
                                if bytes.len() > file_bytes {
                                    break;
                                }
                            }
                            if bytes.len() > file_bytes {
                                return Err(std::io::Error::other("size limit"));
                            }
                            Ok(Some(bytes))
                        })();
                        let text = match text {
                            Ok(Some(text)) => text,
                            Ok(None) => continue,
                            Err(_) => {
                                partial = true;
                                continue;
                            }
                        };
                        let Ok(text) = String::from_utf8(text) else {
                            continue;
                        };
                        if text
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
                        {
                            continue;
                        }
                        let version = fingerprint(text.as_bytes());
                        for (line, range) in crate::editor::line_ranges(&text).enumerate() {
                            let value = &text[range];
                            if cancelled() {
                                break;
                            }
                            if value.contains(&query) {
                                hits.push(Hit {
                                    path: entry.path().to_owned(),
                                    line: line + 1,
                                    excerpt: value.chars().take(160).collect(),
                                    query: query.clone(),
                                    fingerprint: Some(version),
                                    score: 0,
                                });
                                if hits.len() >= 1000 {
                                    partial = true;
                                    break;
                                }
                            }
                        }
                    } else if let Some(score) = rank(entry.path(), &scope, &query) {
                        hits.push(Hit {
                            path: entry.path().to_owned(),
                            line: 1,
                            excerpt: String::new(),
                            query: query.clone(),
                            fingerprint: None,
                            score,
                        });
                    }
                    if last.elapsed() >= Duration::from_millis(40) {
                        if !send(&hits, "Searching") {
                            break;
                        }
                        last = Instant::now();
                    }
                }
                if !cancelled() {
                    if building {
                        let mut index = worker_index.lock().unwrap();
                        if cancelled() {
                            continue;
                        }
                        *index = Some(FilenameIndex {
                            scope,
                            filters,
                            paths: Arc::new(indexed_paths),
                            partial,
                            stale,
                            _watcher: index_watcher,
                        });
                    }
                    send(
                        &hits,
                        if building {
                            "Filename index ready · session snapshot · rebuild for changes"
                        } else if partial {
                            "Partial — limits or unreadable paths"
                        } else {
                            "Complete"
                        },
                    );
                }
            }
        });
        Self {
            index,
            generation,
            latest,
            jobs,
            replies,
        }
    }
    pub fn clear_index(&self) {
        self.cancel();
        *self.index.lock().unwrap() = None;
    }
    pub fn index_compatible(&self, scope: &Path, filters: &Filters) -> bool {
        self.index.try_lock().ok().is_some_and(|index| {
            index
                .as_ref()
                .is_some_and(|index| index.scope == scope && index.filters == *filters)
        })
    }
    pub fn index_status(&self, scope: &Path) -> Option<String> {
        self.index.try_lock().ok().and_then(|index| {
            index
                .as_ref()
                .filter(|index| index.scope == scope)
                .map(|index| {
                    format!(
                        "{} filenames · {}snapshot",
                        index.paths.len(),
                        if index.stale.load(Ordering::Relaxed) {
                            "stale · rebuild "
                        } else if index.partial {
                            "partial "
                        } else {
                            ""
                        }
                    )
                })
        })
    }
    pub fn cancel(&self) -> u64 {
        *self.latest.lock().unwrap() = None;
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }
    #[cfg(test)]
    pub fn start(
        &self,
        scope: PathBuf,
        query: String,
        content: bool,
        ignored: bool,
    ) -> Result<u64, String> {
        self.start_filtered(
            scope,
            query,
            content,
            Filters {
                ignored,
                file_bytes: 1024 * 1024,
                ..Default::default()
            },
        )
    }
    pub fn start_filtered(
        &self,
        scope: PathBuf,
        query: String,
        content: bool,
        filters: Filters,
    ) -> Result<u64, String> {
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *self.latest.lock().unwrap() = Some((id, scope, query, content, filters));
        let _ = self.jobs.try_send(());
        Ok(id)
    }
}
impl Drop for Search {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "release search CPU benchmark; excludes UI presentation"]
    fn benchmark_indexed_hundred_thousand_paths() {
        let root = tempfile::tempdir().unwrap();
        let scope = root.path().to_path_buf();
        let search = Search::new(Default::default());
        *search.index.lock().unwrap() = Some(FilenameIndex {
            scope: scope.clone(),
            filters: Filters::default(),
            partial: false,
            stale: Arc::new(AtomicBool::new(false)),
            _watcher: None,
            paths: Arc::new(
                (0..100_000)
                    .map(|i| scope.join(format!("group{}/item{i:06}.rs", i / 1000)))
                    .collect(),
            ),
        });
        let mut samples = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            let id = search
                .start(scope.clone(), "item099999".into(), false, false)
                .unwrap();
            let result = terminal(&search, id);
            assert_eq!(result.hits[0].path, scope.join("group99/item099999.rs"));
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "100k indexed paths,20 worker queries: p50={:.3}ms p95={:.3}ms max={:.3}ms; excludes render/present",
            samples[10], samples[18], samples[19]
        );
    }

    #[test]
    fn selected_scope_index_becomes_stale_without_rebuilding_or_expanding() {
        let root = tempfile::tempdir().unwrap();
        let scope = root.path().join("scope");
        std::fs::create_dir(&scope).unwrap();
        std::fs::write(scope.join("before.rs"), "one").unwrap();
        let search = Search::new(Default::default());
        let id = search
            .start(scope.clone(), String::new(), false, false)
            .unwrap();
        terminal(&search, id);
        assert!(!search.index_status(&scope).unwrap().contains("stale"));
        std::fs::write(root.path().join("outside.rs"), "outside").unwrap();
        std::fs::write(scope.join("after.rs"), "two").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !search.index_status(&scope).unwrap().contains("stale") {
            assert!(
                Instant::now() < deadline,
                "selected scope change must invalidate snapshot"
            );
            thread::sleep(Duration::from_millis(20));
        }
        let id = search
            .start(scope.clone(), "after".into(), false, false)
            .unwrap();
        let old = terminal(&search, id);
        assert!(old.hits.is_empty());
        assert!(old.state.contains("snapshot stale"));
        let id = search
            .start(scope.clone(), String::new(), false, false)
            .unwrap();
        terminal(&search, id);
        let id = search
            .start(scope.clone(), "after".into(), false, false)
            .unwrap();
        assert_eq!(terminal(&search, id).hits.len(), 1);
        let id = search.start(scope, "outside".into(), false, false).unwrap();
        assert!(terminal(&search, id).hits.is_empty());
    }

    #[test]
    fn filename_index_is_explicit_scoped_filtered_and_removable() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("alpha.rs"), "one").unwrap();
        std::fs::write(root.path().join("beta.rs"), "two").unwrap();
        let search = Search::new(Default::default());
        let scope = root.path().to_path_buf();
        assert!(search.index_status(&scope).is_none());
        let id = search
            .start(scope.clone(), "alpha".into(), false, false)
            .unwrap();
        assert_eq!(terminal(&search, id).hits.len(), 1);
        assert!(search.index_status(&scope).is_none());
        let id = search
            .start(scope.clone(), String::new(), false, false)
            .unwrap();
        terminal(&search, id);
        assert!(
            search
                .index_status(&scope)
                .unwrap()
                .starts_with("2 filenames")
        );
        let defaults = Filters::default();
        assert!(search.index_compatible(&scope, &defaults));
        assert!(!search.index_compatible(
            &scope,
            &Filters {
                ignored: true,
                ..defaults
            }
        ));
        std::fs::write(root.path().join("gamma.rs"), "three").unwrap();
        let id = search
            .start(scope.clone(), "gamma".into(), false, false)
            .unwrap();
        let result = terminal(&search, id);
        assert!(result.hits.is_empty());
        assert!(result.state.contains("snapshot"));
        let id = search
            .start(scope.clone(), "gamma".into(), false, true)
            .unwrap();
        assert_eq!(
            terminal(&search, id).hits.len(),
            1,
            "different filters bypass snapshot"
        );
        search.clear_index();
        assert!(search.index_status(&scope).is_none());
        let id = search.start(scope, "gamma".into(), false, false).unwrap();
        assert_eq!(terminal(&search, id).hits.len(), 1);
    }

    fn terminal(search: &Search, id: u64) -> ResultSet {
        loop {
            let result = search.replies.recv_timeout(Duration::from_secs(5)).unwrap();
            if result.generation == id && result.state != "Searching" {
                return result;
            }
        }
    }
    #[test]
    fn explicit_exclusions_metadata_and_ignored_overrides_preserve_scope() {
        let root = tempfile::tempdir().unwrap();
        let scope = root.path().join("scope");
        std::fs::create_dir(&scope).unwrap();
        std::fs::write(scope.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::create_dir(scope.join(".git")).unwrap();
        for path in [
            "inside.txt",
            "ignored.txt",
            "skip.tmp",
            ".hidden",
            ".git/config",
        ] {
            std::fs::write(scope.join(path), "needle").unwrap();
        }
        let outside = root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("escape.txt"), "needle").unwrap();
        std::os::unix::fs::symlink(&outside, scope.join("linked")).unwrap();
        let search = Search::new(crate::wake::Context::default());
        let id = search
            .start_filtered(
                scope.clone(),
                "needle".into(),
                true,
                Filters {
                    exclude: "*.tmp".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let result = terminal(&search, id);
        assert_eq!(result.state, "Complete");
        let paths: Vec<_> = result
            .hits
            .iter()
            .map(|h| h.path.strip_prefix(&scope).unwrap().to_path_buf())
            .collect();
        assert_eq!(
            paths,
            [PathBuf::from(".hidden"), PathBuf::from("inside.txt")]
        );
        let id = search
            .start_filtered(
                scope.clone(),
                "needle".into(),
                true,
                Filters {
                    ignored: true,
                    metadata: true,
                    exclude: "*.tmp".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let result = terminal(&search, id);
        assert_eq!(result.hits.len(), 4);
        assert!(
            result
                .hits
                .iter()
                .all(|h| h.path.starts_with(&scope) && !h.path.starts_with(scope.join("linked")))
        );
    }
    #[test]
    fn content_lines_binary_exclusion_and_size_limit_are_explicit() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("lines"),
            b"\xef\xbb\xbffirst\rneedle\r\nneedle\n",
        )
        .unwrap();
        std::fs::write(root.path().join("binary"), b"needle\x01").unwrap();
        std::fs::write(root.path().join("invalid"), b"needle\xff").unwrap();
        let search = Search::new(crate::wake::Context::default());
        let id = search
            .start_filtered(
                root.path().into(),
                "needle".into(),
                true,
                Default::default(),
            )
            .unwrap();
        let result = terminal(&search, id);
        assert_eq!(result.state, "Complete");
        assert_eq!(
            result.hits.iter().map(|h| h.line).collect::<Vec<_>>(),
            [2, 3]
        );
        std::fs::write(root.path().join("large"), "needle longer than limit").unwrap();
        let id = search
            .start_filtered(
                root.path().into(),
                "needle".into(),
                true,
                Filters {
                    file_bytes: 8,
                    ..Default::default()
                },
            )
            .unwrap();
        let result = terminal(&search, id);
        assert!(result.state.starts_with("Partial"));
        assert!(result.hits.is_empty());
    }
    #[test]
    fn result_cap_and_unavailable_scope_are_explicit() {
        let root = tempfile::tempdir().unwrap();
        for id in 0..1200 {
            std::fs::write(root.path().join(format!("match-{id}")), "").unwrap();
        }
        let search = Search::new(crate::wake::Context::default());
        let old = search
            .start(root.path().into(), "match".into(), false, false)
            .unwrap();
        let final_result = terminal(&search, old);
        assert!(final_result.state.starts_with("Partial"));
        assert_eq!(final_result.hits.len(), 1000);
        let id = search
            .start(root.path().join("absent"), "match".into(), false, false)
            .unwrap();
        let result = terminal(&search, id);
        assert!(result.state.starts_with("Failed"));
        assert!(result.hits.is_empty());
    }
    #[test]
    fn scope_ignore_ranking_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let scope = root.path().join("scope");
        std::fs::create_dir(&scope).unwrap();
        std::fs::write(root.path().join("outside"), "needle").unwrap();
        std::fs::write(scope.join("inside.txt"), "needle\n").unwrap();
        std::fs::write(scope.join(".ignore"), "ignored.txt\n").unwrap();
        std::fs::write(scope.join("ignored.txt"), "needle").unwrap();
        let search = Search::new(crate::wake::Context::default());
        let old = search
            .start(scope.clone(), "old".into(), true, false)
            .unwrap();
        let id = search
            .start(scope.clone(), "needle".into(), true, false)
            .unwrap();
        assert!(id > old);
        loop {
            let result = search.replies.recv_timeout(Duration::from_secs(5)).unwrap();
            if result.generation != id || result.state == "Searching" {
                continue;
            }
            assert_eq!(result.hits.len(), 1);
            assert_eq!(result.hits[0].path, scope.join("inside.txt"));
            assert_eq!(result.state, "Complete");
            assert_eq!(result.hits[0].fingerprint, Some(fingerprint(b"needle\n")));
            break;
        }
        assert!(search.cancel() > id);
        assert!(
            rank(Path::new("/scope/readme"), Path::new("/scope"), "readme")
                > rank(
                    Path::new("/scope/deep/readme-long"),
                    Path::new("/scope"),
                    "readme"
                )
        );
        assert!(rank(Path::new("/scope/readme"), Path::new("/scope"), "rdm").is_some());
    }
}
