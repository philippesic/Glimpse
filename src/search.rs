use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, hash_map::DefaultHasher},
    ffi::OsStr,
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
use unicode_segmentation::UnicodeSegmentation;

const MAX_RESULTS: usize = 1000;

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
    pub duplicate_parents: HashMap<PathBuf, DuplicateParent>,
    pub state: String,
    pub completed: bool,
}
pub struct DuplicateParent {
    pub suffix: String,
    pub compact: String,
    pub ordinal: usize,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Filters {
    pub ignored: bool,
    pub metadata: bool,
    pub cross_mounts: bool,
    pub directory_links: bool,
    pub exclude: String,
    pub file_bytes: usize,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            ignored: false,
            metadata: false,
            cross_mounts: false,
            directory_links: false,
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
    match bytes.strip_prefix(crate::document::BOM) {
        Some(body) => fingerprint_body(body, true),
        None => fingerprint_body(bytes, false),
    }
}
pub fn fingerprint_body(body: &[u8], bom: bool) -> u64 {
    let mut hash = DefaultHasher::new();
    bom.hash(&mut hash);
    body.hash(&mut hash);
    hash.finish()
}
/// Byte ranges of `value` to emphasise for a filename query: the last contiguous occurrence
/// when there is one (so a match in the file name wins over its folders), otherwise the query's
/// characters in order, starting in the file name when they all fit there.
pub fn highlight_ranges(value: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let chars: Vec<_> = value
        .char_indices()
        .flat_map(|(start, c)| {
            c.to_lowercase()
                .map(move |lower| (start, start + c.len_utf8(), lower))
        })
        .collect();
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    if let Some(at) = (0..=chars.len().saturating_sub(wanted.len()))
        .rev()
        .filter(|_| chars.len() >= wanted.len())
        .find(|&at| {
            chars[at..at + wanted.len()]
                .iter()
                .zip(&wanted)
                .all(|((_, _, c), w)| c == w)
        })
    {
        return std::iter::once(chars[at].0..chars[at + wanted.len() - 1].1).collect();
    }
    let name_start = chars
        .iter()
        .rposition(|(_, _, c)| *c == '/')
        .map_or(0, |slash| slash + 1);
    let subsequence = |from: usize| {
        let mut position = from;
        let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
        for expected in &wanted {
            let offset = chars[position..]
                .iter()
                .position(|(_, _, c)| c == expected)?;
            position += offset;
            let (start, end, _) = chars[position];
            if let Some(last) = ranges.last_mut().filter(|range| range.end >= start) {
                last.end = end;
            } else {
                ranges.push(start..end);
            }
            position += 1;
        }
        Some(ranges)
    };
    subsequence(name_start)
        .or_else(|| subsequence(0))
        .unwrap_or_default()
}
fn excerpt(value: &str, query: &str) -> String {
    let found = value.find(query).unwrap_or(0);
    let start = value[..found]
        .char_indices()
        .rev()
        .nth(47)
        .map_or(0, |(offset, _)| offset);
    let text: String = value[start..].chars().take(160).collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        text,
        if start + text.len() < value.len() {
            "…"
        } else {
            ""
        }
    )
}
fn rank(path: &Path, scope: &Path, query: &str) -> Option<i64> {
    let basename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if basename == query {
        return Some(10000);
    }
    if basename.starts_with(query) {
        return Some(8000 - basename.len() as i64);
    }
    if let Some(index) = basename.find(query) {
        return Some(6000 - index as i64);
    }
    let value = path
        .strip_prefix(scope)
        .unwrap_or(path)
        .to_string_lossy()
        .to_lowercase();
    if let Some(index) = value.find(query) {
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
fn shared_grapheme_bytes<'a>(
    left: impl Iterator<Item = &'a str>,
    right: impl Iterator<Item = &'a str>,
) -> usize {
    left.zip(right)
        .take_while(|(left, right)| left == right)
        .map(|(grapheme, _)| grapheme.len())
        .sum()
}

fn shared_path_edge(left: &str, right: &str, from_end: bool) -> usize {
    if left.is_ascii() && right.is_ascii() {
        let equal = |(left, right): &(u8, u8)| left == right;
        if from_end {
            left.bytes()
                .rev()
                .zip(right.bytes().rev())
                .take_while(equal)
                .count()
        } else {
            left.bytes().zip(right.bytes()).take_while(equal).count()
        }
    } else if from_end {
        shared_grapheme_bytes(left.graphemes(true).rev(), right.graphemes(true).rev())
    } else {
        shared_grapheme_bytes(left.graphemes(true), right.graphemes(true))
    }
}

fn duplicate_parents(hits: &[Hit], scope: &Path) -> HashMap<PathBuf, DuplicateParent> {
    let mut by_name: HashMap<&OsStr, HashSet<&OsStr>> = HashMap::new();
    for hit in hits {
        if let Some(name) = hit.path.file_name() {
            by_name
                .entry(name)
                .or_default()
                .insert(hit.path.as_os_str());
        }
    }
    let mut parents = HashMap::new();
    for paths in by_name.values().filter(|paths| paths.len() > 1) {
        let mut counts = HashMap::new();
        for &path in paths {
            for suffix in parent_suffixes(Path::new(path)) {
                *counts.entry(suffix.as_os_str()).or_insert(0) += 1;
            }
        }
        let mut group = paths
            .iter()
            .map(|&path| {
                let path = Path::new(path);
                let parent = path.parent().unwrap_or(path);
                (
                    path,
                    crate::paths::display(parent.strip_prefix(scope).unwrap_or(parent)),
                )
            })
            .collect::<Vec<_>>();
        group.sort_unstable_by_key(|(path, _)| path.as_os_str());
        let first = group[0].1.as_str();
        let prefix = group
            .iter()
            .skip(1)
            .fold(first.len(), |prefix, (_, other)| {
                shared_path_edge(&first[..prefix], other, false)
            });
        let suffix_bytes = group
            .iter()
            .skip(1)
            .fold(first.len() - prefix, |suffix, (_, other)| {
                shared_path_edge(&first[first.len() - suffix..], &other[prefix..], true)
            });
        for (index, (path, parent)) in group.into_iter().enumerate() {
            if let Some(suffix) = parent_suffixes(path)
                .filter(|suffix| counts.get(suffix.as_os_str()) == Some(&1))
                .last()
            {
                let start = if prefix == 0 { "" } else { "…" };
                let end = if suffix_bytes == 0 { "" } else { "…" };
                let difference = &parent[prefix..parent.len() - suffix_bytes];
                let compact = if parent.is_empty() {
                    "Search folder".into()
                } else {
                    format!("{start}{difference}{end}")
                };
                parents.insert(
                    path.to_owned(),
                    DuplicateParent {
                        suffix: crate::paths::display(suffix),
                        compact,
                        ordinal: index + 1,
                    },
                );
            }
        }
    }
    parents
}

fn parent_suffixes(path: &Path) -> impl Iterator<Item = &Path> {
    let mut parts = path.parent().unwrap_or(path).components();
    std::iter::from_fn(move || {
        let suffix = parts.as_path();
        parts.next().map(|_| suffix)
    })
}

fn indexed_hits(
    paths: &[PathBuf],
    scope: &Path,
    query: &str,
    folded_query: &str,
    cancelled: impl Fn() -> bool,
) -> Result<(Vec<Hit>, bool), usize> {
    #[derive(Eq, Ord, PartialEq, PartialOrd)]
    struct RankedPath<'a> {
        score: Reverse<i64>,
        path: &'a Path,
    }

    let mut best = BinaryHeap::with_capacity(MAX_RESULTS);
    let mut truncated = false;
    for (processed, path) in paths.iter().enumerate() {
        if cancelled() {
            return Err(processed);
        }
        let Some(score) = rank(path, scope, folded_query) else {
            continue;
        };
        let ranked = RankedPath {
            score: Reverse(score),
            path,
        };
        if best.len() < MAX_RESULTS {
            best.push(ranked);
        } else if let Some(mut worst) = best.peek_mut() {
            truncated = true;
            if ranked < *worst {
                *worst = ranked;
            }
        }
    }
    if cancelled() {
        return Err(paths.len());
    }
    let hits = best
        .into_iter()
        .map(|ranked| Hit {
            path: ranked.path.to_path_buf(),
            line: 1,
            excerpt: String::new(),
            query: query.to_owned(),
            fingerprint: None,
            score: ranked.score.0,
        })
        .collect();
    Ok((hits, truncated))
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
                #[cfg(feature = "test-support")]
                crate::metrics::delay("SEARCH");
                let cancelled = || cancel.load(Ordering::Relaxed) != id;
                let send = |hits: &[Hit], state: &str, completed: bool| {
                    let mut result = ResultSet {
                        generation: id,
                        hits: ordered(hits),
                        duplicate_parents: duplicate_parents(hits, &scope),
                        state: state.into(),
                        completed,
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
                let folded_query = query.to_lowercase();
                if !content && !building {
                    let cached = worker_index
                        .lock()
                        .unwrap()
                        .as_ref()
                        .filter(|index| index.scope == scope && index.filters == filters)
                        .map(|index| (index.paths.clone(), index.partial, index.stale.clone()));
                    if let Some((paths, partial, stale)) = cached {
                        crate::metrics::event("search-indexed-start", id, paths.len() as u64);
                        let report_abandoned = |processed| {
                            crate::metrics::event("search-indexed-abandoned", id, processed as u64);
                        };
                        let (hits, truncated) =
                            match indexed_hits(&paths, &scope, &query, &folded_query, cancelled) {
                                Ok(hits) => hits,
                                Err(processed) => {
                                    report_abandoned(processed);
                                    continue;
                                }
                            };
                        let state = match (stale.load(Ordering::Relaxed), partial || truncated) {
                            (true, true) => {
                                "Indexed snapshot stale · partial results · rebuild for current results"
                            }
                            (true, false) => "Indexed snapshot stale · rebuild for current results",
                            (false, true) => {
                                "Indexed snapshot · partial results · rebuild for changes"
                            }
                            (false, false) => "Indexed snapshot · rebuild for changes",
                        };
                        if !send(&hits, state, true) && cancelled() {
                            report_abandoned(paths.len());
                        }
                        continue;
                    }
                }
                let stale = Arc::new(AtomicBool::new(false));
                let mut index_watcher = None;
                use notify::Watcher;
                if building {
                    let dirty = stale.clone();
                    let repaint = ctx.clone();
                    if let Ok(watcher) = notify::RecommendedWatcher::new(
                        move |event: notify::Result<notify::Event>| {
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
                        },
                        notify::Config::default().with_follow_symlinks(filters.directory_links),
                    ) {
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
                    send(&[], "Failed — scope unavailable", true);
                    continue;
                }
                let mut exclusions = ignore::overrides::OverrideBuilder::new(&scope);
                let invalid = filters
                    .exclude
                    .lines()
                    .filter(|s| !s.trim().is_empty())
                    .find_map(|line| exclusions.add(&format!("!{line}")).err());
                if let Some(error) = invalid {
                    send(&[], &format!("Failed — invalid exclusion: {error}"), true);
                    continue;
                }
                let exclusions = match exclusions.build() {
                    Ok(value) => value,
                    Err(error) => {
                        send(&[], &format!("Failed — {error}"), true);
                        continue;
                    }
                };
                let boundaries = if filters.cross_mounts {
                    None
                } else {
                    match std::fs::canonicalize(&scope)
                        .map_err(|error| error.to_string())
                        .and_then(|scope| {
                            crate::locations::mount_points().map(|points| (scope, points))
                        }) {
                        Ok((resolved, mounts)) => {
                            let outside = mounts
                                .into_iter()
                                .filter(|path| !resolved.starts_with(path))
                                .collect::<std::collections::HashSet<_>>();
                            Some((resolved, outside))
                        }
                        Err(error) => {
                            send(
                                &[],
                                &format!("Failed — cannot validate mount boundaries: {error}"),
                                true,
                            );
                            continue;
                        }
                    }
                };
                let walked_scope = scope.clone();
                let metadata = filters.metadata;
                let directory_links = filters.directory_links;
                let incomplete = Arc::new(AtomicBool::new(false));
                let skipped_entry = incomplete.clone();
                let file_bytes = filters.file_bytes.clamp(1, 10 * 1024 * 1024);
                let walker = ignore::WalkBuilder::new(&scope)
                    .hidden(false)
                    .git_ignore(!filters.ignored)
                    .ignore(!filters.ignored)
                    .git_exclude(!filters.ignored)
                    .require_git(false)
                    .follow_links(directory_links)
                    .same_file_system(!filters.cross_mounts)
                    .overrides(exclusions)
                    .filter_entry(move |entry| {
                        if entry.depth() == 0 {
                            return true;
                        }
                        if !metadata
                            && [".git", ".hg", ".svn"]
                                .iter()
                                .any(|name| entry.file_name() == *name)
                        {
                            return false;
                        }
                        if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                            return !entry.path_is_symlink();
                        }
                        let Some((scope, mounts)) = &boundaries else {
                            return true;
                        };
                        if directory_links {
                            let Ok(target) = std::fs::canonicalize(entry.path()) else {
                                skipped_entry.store(true, Ordering::Relaxed);
                                return false;
                            };
                            return !target.ancestors().any(|path| mounts.contains(path));
                        }
                        entry
                            .path()
                            .strip_prefix(&walked_scope)
                            .is_ok_and(|relative| !mounts.contains(&scope.join(relative)))
                    })
                    .build();
                for entry in walker {
                    if cancelled() {
                        break;
                    }
                    count += 1;
                    if count > 100_000 || hits.len() >= MAX_RESULTS {
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
                    if entry.file_type().is_some_and(|kind| kind.is_dir())
                        && let Some(watcher) = &mut index_watcher
                        && watcher
                            .watch(entry.path(), notify::RecursiveMode::NonRecursive)
                            .is_err()
                    {
                        stale.store(true, Ordering::Relaxed);
                    }
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
                                    excerpt: excerpt(value, &query),
                                    query: query.clone(),
                                    fingerprint: Some(version),
                                    score: 0,
                                });
                                if hits.len() >= MAX_RESULTS {
                                    partial = true;
                                    break;
                                }
                            }
                        }
                    } else if let Some(score) = rank(entry.path(), &scope, &folded_query) {
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
                        if !send(&hits, "Searching", false) {
                            break;
                        }
                        last = Instant::now();
                    }
                }
                partial |= incomplete.load(Ordering::Relaxed);
                if !cancelled() {
                    if building {
                        let mut index = worker_index.lock().unwrap();
                        if cancelled() {
                            continue;
                        }
                        *index = Some(FilenameIndex {
                            scope: scope.clone(),
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
                        true,
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
                        match (index.stale.load(Ordering::Relaxed), index.partial) {
                            (true, true) => "partial · stale · rebuild ",
                            (true, false) => "stale · rebuild ",
                            (false, true) => "partial ",
                            (false, false) => "",
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
        match self.jobs.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(id),
            Err(mpsc::TrySendError::Disconnected(())) => {
                Err("Search worker unavailable; retry after reopening Glimpse".into())
            }
        }
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
    fn indexed_hits_keep_the_best_paths_and_abandon_work() {
        let scope = Path::new("/fixtures");
        let paths: Vec<_> = (0..1024)
            .rev()
            .map(|index| scope.join(format!("Item{index:04}.rs")))
            .collect();
        let (hits, truncated) = indexed_hits(&paths, scope, "ITEM", "item", || false).unwrap();
        assert!(truncated);
        let hits = ordered(&hits);
        assert_eq!(hits.len(), MAX_RESULTS);
        assert_eq!(hits[0].path, scope.join("Item0000.rs"));
        assert_eq!(hits[MAX_RESULTS - 1].path, scope.join("Item0999.rs"));
        assert_eq!(hits[0].query, "ITEM");

        let checks = std::cell::Cell::new(0);
        assert!(matches!(
            indexed_hits(&paths, scope, "ITEM", "item", || {
                checks.set(checks.get() + 1);
                checks.get() == 10
            }),
            Err(9)
        ));
        assert_eq!(checks.get(), 10);
    }

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

#[cfg(test)]
mod presentation_tests {
    use super::*;
    #[test]
    fn unicode_highlights_and_long_line_context_retain_match_boundaries() {
        let value = "İmage/文档/native.txt";
        let ranges = highlight_ranges(value, "İ文nat");
        assert!(!ranges.is_empty());
        for range in ranges {
            assert!(value.is_char_boundary(range.start));
            assert!(value.is_char_boundary(range.end));
        }
        let line = format!("{}needle α{}", "文".repeat(300), "tail".repeat(100));
        let shown = excerpt(&line, "needle α");
        assert!(shown.contains("needle α"));
        assert!(shown.chars().count() <= 162);
        assert!(shown.starts_with('…') && shown.ends_with('…'));
    }
    #[test]
    fn filename_highlights_prefer_a_whole_match_in_the_file_name() {
        let value = "project-a/src/config/settings.toml";
        let whole = highlight_ranges(value, "Settings");
        assert_eq!((whole.len(), whole.first()), (1, Some(&(21..29))));
        assert_eq!(highlight_ranges(value, "stt"), vec![21..22, 23..25]);
        assert_eq!(highlight_ranges(value, "pac"), vec![0..1, 8..9, 12..13]);
        assert!(highlight_ranges(value, "zz").is_empty());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod mount_tests {
    use super::*;
    #[test]
    fn same_device_mounts_and_symlinks_require_independent_search_opt_ins() {
        use std::os::unix::fs::{MetadataExt, symlink};
        if let Some(root) = std::env::var_os("GLIMPSE_MOUNT_FIXTURE") {
            let root = PathBuf::from(root);
            let parent_namespace = std::env::var_os("GLIMPSE_PARENT_MOUNT_NAMESPACE").unwrap();
            assert_ne!(
                std::fs::read_link("/proc/self/ns/mnt").unwrap().as_os_str(),
                parent_namespace
            );
            assert!(
                std::process::Command::new("mount")
                    .arg("--bind")
                    .arg(root.join("outside"))
                    .arg(root.join("scope/mounted"))
                    .status()
                    .unwrap()
                    .success()
            );
            assert_eq!(
                std::fs::metadata(root.join("scope")).unwrap().dev(),
                std::fs::metadata(root.join("scope/mounted")).unwrap().dev()
            );
            assert!(root.join("scope/mounted/needle-outside.txt").is_file());
            let search = Search::new(crate::wake::Context::default());
            let scope = root.join("scope");
            let terminal = |generation| loop {
                let result = search.replies.recv_timeout(Duration::from_secs(5)).unwrap();
                if result.generation == generation && result.completed {
                    break result;
                }
            };
            for (cross_mounts, directory_links) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                let filters = Filters {
                    cross_mounts,
                    directory_links,
                    ..Default::default()
                };
                let mut expected = vec![scope.join("needle-visible.txt")];
                if cross_mounts {
                    expected.push(scope.join("mounted/needle-outside.txt"));
                }
                if directory_links {
                    expected.push(scope.join("alias/needle-outside.txt"));
                }
                if cross_mounts && directory_links {
                    expected.push(scope.join("through-mount/needle-outside.txt"));
                }
                expected.sort();
                for indexed in [false, true] {
                    search.clear_index();
                    if indexed {
                        let id = search
                            .start_filtered(scope.clone(), String::new(), false, filters.clone())
                            .unwrap();
                        terminal(id);
                        assert!(search.index_compatible(&scope, &filters));
                    }
                    let id = search
                        .start_filtered(scope.clone(), "needle".into(), false, filters.clone())
                        .unwrap();
                    let result = terminal(id);
                    let mut paths = result
                        .hits
                        .into_iter()
                        .map(|hit| hit.path)
                        .collect::<Vec<_>>();
                    paths.sort();
                    assert_eq!(paths, expected, "{}", result.state);
                    if directory_links {
                        assert!(
                            result.state.to_lowercase().contains("partial"),
                            "{}",
                            result.state
                        );
                    }
                }
            }
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("scope/mounted")).unwrap();
        std::fs::create_dir(root.path().join("outside")).unwrap();
        std::fs::write(root.path().join("scope/needle-visible.txt"), b"inside").unwrap();
        std::fs::write(root.path().join("outside/needle-outside.txt"), b"outside").unwrap();
        symlink(root.path().join("outside"), root.path().join("scope/alias")).unwrap();
        symlink(
            root.path().join("scope/mounted"),
            root.path().join("scope/through-mount"),
        )
        .unwrap();
        symlink(root.path().join("scope"), root.path().join("scope/cycle")).unwrap();
        symlink(
            root.path().join("outside/needle-outside.txt"),
            root.path().join("scope/needle-file-link.txt"),
        )
        .unwrap();
        let result = std::process::Command::new("unshare")
            .args(["--user","--map-root-user","--mount","--propagation","private","--"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact","search::mount_tests::same_device_mounts_and_symlinks_require_independent_search_opt_ins","--nocapture"])
            .env("GLIMPSE_MOUNT_FIXTURE",root.path())
            .env("GLIMPSE_PARENT_MOUNT_NAMESPACE",std::fs::read_link("/proc/self/ns/mnt").unwrap())
            .output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
