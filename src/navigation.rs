mod ui;
use egui::{self, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const ROW_HEIGHT: f32 = 28.0;
const GESTURE_DISTANCE: f32 = 65.0;

pub use crate::directory::Entry;

enum Listing {
    Loading,
    Ready(Vec<Entry>),
    Failed(String),
}

#[derive(Clone)]
struct Row {
    path: PathBuf,
    directory: bool,
    symlink: bool,
    depth: usize,
    message: Option<String>,
}

struct RowCache {
    folder: PathBuf,
    query: String,
    show_hidden: bool,
    revision: u64,
    rows: Arc<Vec<Row>>,
    bytes: usize,
}

pub struct Navigator {
    pub current: PathBuf,
    pub filter: String,
    pub row_height: f32,
    pub show_hidden: bool,
    home: PathBuf,
    columns: Vec<PathBuf>,
    active: usize,
    selections: HashMap<PathBuf, PathBuf>,
    folders: HashMap<PathBuf, Listing>,
    generations: HashMap<PathBuf, u64>,
    notices: HashMap<PathBuf, String>,
    ages: HashMap<PathBuf, u64>,
    clock: u64,
    row_cache: Vec<RowCache>,
    requests: Vec<PathBuf>,
    vertical: HashMap<(PathBuf, bool), f32>,
    follow: bool,
    reveal_selection: bool,
    gesture_distance: f32,
    gesture_latched: bool,
    last_gesture: f64,
    gesture_phased: bool,
    gesture_step: Option<bool>,
    pub focused: bool,
    pub reduce_motion: bool,
    last_shown: Option<PathBuf>,
    transition_started: f64,
    transition_direction: f32,
    pending_first_batch: HashSet<PathBuf>,
    navigation_started: Option<std::time::Instant>,
}

#[derive(Default)]
pub struct NavigationAction {
    pub open: Option<(PathBuf, bool)>,
    pub edit: bool,
    pub back: Option<bool>,
}

impl Navigator {
    pub fn new(home: PathBuf, current: PathBuf) -> Self {
        let mut nav = Self {
            current: current.clone(),
            row_height: ROW_HEIGHT,
            filter: String::new(),
            show_hidden: true,
            home,
            columns: vec![current.clone()],
            active: 0,
            selections: HashMap::new(),
            folders: HashMap::new(),
            generations: HashMap::new(),
            notices: HashMap::new(),
            ages: HashMap::new(),
            clock: 0,
            row_cache: Vec::new(),
            requests: Vec::new(),
            vertical: HashMap::new(),
            follow: true,
            reveal_selection: false,
            gesture_distance: 0.0,
            gesture_latched: false,
            last_gesture: f64::NEG_INFINITY,
            gesture_phased: false,
            gesture_step: None,
            focused: true,
            reduce_motion: false,
            last_shown: None,
            transition_started: f64::NEG_INFINITY,
            transition_direction: 1.,
            pending_first_batch: HashSet::new(),
            navigation_started: None,
        };
        nav.request(current);
        nav
    }

    fn request(&mut self, path: PathBuf) {
        if !self.folders.contains_key(&path)
            || self
                .notices
                .get(&path)
                .is_some_and(|v| v.starts_with("Cancelled"))
        {
            self.row_cache.retain(|cache| cache.folder != path);
            self.notices.remove(&path);
            self.folders.insert(path.clone(), Listing::Loading);
            self.requests.push(path);
        }
    }

    pub fn usable(&self) -> bool {
        match self.folders.get(&self.current) {
            Some(Listing::Ready(entries)) => {
                !entries.is_empty() || !self.generations.contains_key(&self.current)
            }
            Some(Listing::Failed(_)) => true,
            _ => false,
        }
    }
    pub fn requested_folders(&self) -> Vec<PathBuf> {
        vec![self.current.clone()]
    }
    pub fn cancel_inactive(&mut self) {
        let visible = self.requested_folders();
        let cancelled: Vec<_> = self
            .generations
            .keys()
            .filter(|p| !visible.contains(p))
            .cloned()
            .collect();
        for path in cancelled {
            self.row_cache.retain(|cache| cache.folder != path);
            self.generations.remove(&path);
            self.notices
                .insert(path, "Cancelled — activate folder to retry".into());
        }
    }
    pub fn begin(&mut self, path: PathBuf, generation: u64) {
        self.row_cache.retain(|cache| cache.folder != path);
        self.generations.insert(path.clone(), generation);
        self.notices.insert(path.clone(), "Loading…".into());
        self.pending_first_batch.insert(path.clone());
        self.folders
            .entry(path)
            .or_insert_with(|| Listing::Ready(Vec::new()));
    }
    pub fn batch(&mut self, batch: crate::directory::Batch) {
        if self.generations.get(&batch.path) != Some(&batch.generation) {
            return;
        }
        if self.pending_first_batch.remove(&batch.path) {
            self.folders
                .insert(batch.path.clone(), Listing::Ready(Vec::new()));
        }
        if let Some(Listing::Ready(entries)) = self.folders.get_mut(&batch.path) {
            let order = |a: &Entry, b: &Entry| {
                b.directory
                    .cmp(&a.directory)
                    .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
            };
            let mut incoming = batch.entries;
            incoming.sort_by(order);
            let mut merged = Vec::with_capacity(entries.len() + incoming.len());
            let mut old = std::mem::take(entries).into_iter().peekable();
            let mut new = incoming.into_iter().peekable();
            while let (Some(a), Some(b)) = (old.peek(), new.peek()) {
                if order(a, b).is_le() {
                    merged.push(old.next().unwrap());
                } else {
                    merged.push(new.next().unwrap());
                }
            }
            merged.extend(old);
            merged.extend(new);
            *entries = merged;
        }
        if batch.done {
            self.generations.remove(&batch.path);
            self.notices.remove(&batch.path);
        }
        if let Some(error) = batch.error {
            self.notices.insert(batch.path.clone(), error);
        }
        self.clock += 1;
        self.ages.insert(batch.path, self.clock);
        self.trim_cache();
    }
    fn trim_cache(&mut self) {
        loop {
            let bytes: usize = self
                .folders
                .values()
                .map(|listing| match listing {
                    Listing::Ready(entries) => entries
                        .iter()
                        .map(|e| e.path.as_os_str().len() * 2 + std::mem::size_of::<Entry>())
                        .sum(),
                    _ => 0,
                })
                .sum();
            if bytes <= 32 * 1024 * 1024 && self.folders.len() <= 64 {
                break;
            }
            let victim = self
                .folders
                .keys()
                .filter(|p| **p != self.current)
                .min_by_key(|p| self.ages.get(*p).copied().unwrap_or(0))
                .cloned();
            let Some(path) = victim else {
                break;
            };
            self.folders.remove(&path);
            self.notices.remove(&path);
            self.ages.remove(&path);
            self.generations.remove(&path);
        }
        self.row_cache
            .retain(|cache| self.folders.contains_key(&cache.folder));
        if self.vertical.len() > 512 {
            self.vertical.retain(|(p, _), _| self.columns.contains(p));
        }
        if self.selections.len() > 512 {
            self.selections.retain(|p, _| self.columns.contains(p));
        }
    }
    pub fn selection(&self) -> Option<PathBuf> {
        self.selections.get(&self.current).cloned()
    }

    pub fn take_requests(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.requests)
    }

    pub fn receive(&mut self, path: PathBuf, entries: Result<Vec<Entry>, String>) {
        self.clock += 1;
        self.ages.insert(path.clone(), self.clock);
        self.folders.insert(
            path,
            match entries {
                Ok(entries) => Listing::Ready(entries),
                Err(error) => Listing::Failed(error),
            },
        );
        self.trim_cache();
    }

    pub fn resume(&mut self) {
        self.focused = true;
    }

    fn activate_column(&mut self, index: usize) {
        self.active = index;
        self.current = self.columns[index].clone();
        self.resume();
        self.follow = true;
        self.request(self.current.clone());
    }

    pub fn navigate(&mut self, path: PathBuf) {
        self.navigation_started = Some(std::time::Instant::now());
        if let Some(index) = self.columns.iter().position(|p| p == &path) {
            self.activate_column(index);
            return;
        }
        // Retain the common ancestor columns; a sibling replaces the old branch.
        if let Some(index) = self.columns.iter().rposition(|p| path.starts_with(p)) {
            self.columns.truncate(index + 1);
            let mut descendants: Vec<_> = path
                .ancestors()
                .take_while(|p| *p != self.columns[index])
                .map(Path::to_path_buf)
                .collect();
            descendants.reverse();
            for child in descendants {
                let parent = self.columns.last().unwrap().clone();
                self.selections.insert(parent, child.clone());
                self.columns.push(child);
            }
        } else {
            self.columns = vec![path];
        }
        self.activate_column(self.columns.len() - 1);
    }

    pub fn home(&mut self) {
        self.columns = vec![self.home.clone()];
        self.activate_column(0);
    }

    pub fn up(&mut self) {
        if self.active > 0 {
            self.activate_column(self.active - 1);
        } else if let Some(parent) = self.current.parent().map(Path::to_path_buf) {
            self.selections.insert(parent.clone(), self.current.clone());
            self.columns.insert(0, parent);
            self.activate_column(0);
        } else {
            self.resume();
        }
    }

    pub fn forward(&mut self) {
        let selected = self.selections.get(&self.current).cloned();
        let folder = selected.filter(|path| {
            matches!(self.folders.get(&self.current), Some(Listing::Ready(entries))
                if entries.iter().any(|e| e.path == *path && e.directory))
        });
        if let Some(folder) = folder {
            self.navigate(folder);
        } else if self.active + 1 < self.columns.len() {
            self.activate_column(self.active + 1);
        } else {
            self.resume();
        }
    }

    pub fn refresh_visible(&mut self) {
        for path in self.requested_folders() {
            if !self.generations.contains_key(&path) && !self.requests.contains(&path) {
                self.requests.push(path);
            }
        }
    }

    pub fn refresh(&mut self) {
        self.row_cache.retain(|cache| cache.folder != self.current);
        self.folders.insert(self.current.clone(), Listing::Loading);
        self.requests.push(self.current.clone());
    }

    fn select_file(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.navigate(parent.to_path_buf());
            self.selections
                .insert(parent.to_path_buf(), path.to_path_buf());
        }
    }

    pub fn reveal_document(&mut self, path: &Path) {
        if path.parent() == Some(self.current.as_path())
            && self
                .selections
                .get(&self.current)
                .is_none_or(|selected| selected != path)
        {
            self.selections
                .insert(self.current.clone(), path.to_path_buf());
            self.reveal_selection = true;
        }
    }

    pub fn leave_for_editor(&mut self) -> bool {
        self.focused = false;
        false
    }

    fn cached_rows(&mut self, folder: &Path) -> Arc<Vec<Row>> {
        let revision = self.ages.get(folder).copied().unwrap_or(0);
        if let Some(index) = self
            .row_cache
            .iter()
            .position(|cache| cache.folder == folder)
        {
            let cache = self.row_cache.remove(index);
            if cache.query == self.filter
                && cache.show_hidden == self.show_hidden
                && cache.revision == revision
            {
                let rows = cache.rows.clone();
                self.row_cache.push(cache);
                return rows;
            }
        }
        let query = self.filter.to_lowercase();
        let rows: Arc<Vec<Row>> = Arc::new(
            self.rows(folder)
                .into_iter()
                .filter(|row| {
                    if row.message.is_some() {
                        return true;
                    }
                    let name = crate::name(&row.path);
                    (self.show_hidden || !name.starts_with('.'))
                        && (query.is_empty() || name.to_lowercase().contains(&query))
                })
                .collect(),
        );
        let bytes = rows
            .iter()
            .map(|row| {
                std::mem::size_of::<Row>()
                    + row.path.as_os_str().len() * 2
                    + row.message.as_ref().map_or(0, String::len)
            })
            .sum();
        // Retain nearby locations without letting derived rows grow with the navigation history.
        while !self.row_cache.is_empty()
            && (self.row_cache.len() >= 4
                || self
                    .row_cache
                    .iter()
                    .map(|cache| cache.bytes)
                    .sum::<usize>()
                    + bytes
                    > 8 * 1024 * 1024)
        {
            self.row_cache.remove(0);
        }
        self.row_cache.push(RowCache {
            folder: folder.to_path_buf(),
            query: self.filter.clone(),
            show_hidden: self.show_hidden,
            revision,
            rows: rows.clone(),
            bytes,
        });
        rows
    }

    fn rows(&self, folder: &Path) -> Vec<Row> {
        match self.folders.get(folder) {
            Some(Listing::Ready(entries)) if !entries.is_empty() => entries
                .iter()
                .map(|entry| Row {
                    path: entry.path.clone(),
                    directory: entry.directory,
                    symlink: entry.symlink,
                    depth: 0,
                    message: None,
                })
                .collect(),
            listing => vec![Row {
                path: folder.to_path_buf(),
                directory: false,
                symlink: false,
                depth: 0,
                message: Some(match listing {
                    Some(Listing::Ready(_)) => self
                        .notices
                        .get(folder)
                        .cloned()
                        .unwrap_or_else(|| "Empty folder".into()),
                    Some(Listing::Failed(error)) => format!("Cannot list folder: {error}"),
                    Some(Listing::Loading) => "Loading…".into(),
                    None => "Not cached — activate folder".into(),
                }),
            }],
        }
    }

    fn gesture(&mut self, delta: Vec2, phase: egui::TouchPhase, time: f64) -> bool {
        if phase == egui::TouchPhase::Start {
            self.gesture_phased = true;
        }
        if phase == egui::TouchPhase::Start
            || (!self.gesture_phased && time - self.last_gesture > 0.22)
        {
            self.gesture_distance = 0.0;
            self.gesture_latched = false;
        }
        self.last_gesture = time;
        let horizontal = delta.x.abs() > 1.0 && delta.x.abs() > delta.y.abs() * 1.75;
        if horizontal && !self.gesture_latched {
            if self.gesture_distance.signum() != delta.x.signum() {
                self.gesture_distance = 0.0;
            }
            self.gesture_distance += delta.x;
            if self.gesture_distance.abs() >= GESTURE_DISTANCE {
                self.gesture_step = Some(self.gesture_distance > 0.0);
                self.gesture_latched = true;
            }
        } else if !horizontal {
            self.gesture_distance = 0.0;
        }
        if matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
            self.gesture_distance = 0.0;
            // Keep the latch through momentum/end events; a new touch Start releases it.
        }
        horizontal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filtered_rows_reuse_cache_and_invalidate_for_settings_and_batches() {
        let root = PathBuf::from("/home/me");
        let mut nav = nav();
        nav.receive(
            root.clone(),
            Ok(vec![
                file("/home/me/.hidden"),
                file("/home/me/Alpha"),
                file("/home/me/beta"),
            ]),
        );
        nav.selections.insert(root.clone(), root.join("Alpha"));
        nav.vertical.insert((root.clone(), false), 73.);
        let all = nav.cached_rows(&root);
        assert!(Arc::ptr_eq(&all, &nav.cached_rows(&root)));
        nav.cancel_inactive();
        assert!(Arc::ptr_eq(&all, &nav.cached_rows(&root)));
        nav.show_hidden = false;
        assert_eq!(nav.cached_rows(&root).len(), 2);
        nav.filter = "ALP".into();
        let filtered = nav.cached_rows(&root);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].path, root.join("Alpha"));
        nav.begin(root.clone(), 4);
        assert_eq!(nav.cached_rows(&root).len(), 1);
        nav.batch(crate::directory::Batch {
            path: root.clone(),
            generation: 4,
            entries: vec![file("/home/me/Alpine")],
            done: true,
            error: None,
        });
        assert_eq!(nav.cached_rows(&root)[0].path, root.join("Alpine"));
        assert_eq!(nav.selection(), Some(root.join("Alpha")));
        assert_eq!(nav.vertical[&(root.clone(), false)], 73.);
        nav.receive(root.clone(), Err("permission denied".into()));
        assert!(
            nav.cached_rows(&root)[0]
                .message
                .as_ref()
                .unwrap()
                .contains("permission denied")
        );
    }

    #[test]
    fn nearby_folder_rows_survive_switches_and_unrelated_batches() {
        let mut nav = nav();
        let root = PathBuf::from("/home/me");
        let child = PathBuf::from("/home/me/a");
        let parent_rows = nav.cached_rows(&root);
        nav.navigate(child.clone());
        let child_rows = nav.cached_rows(&child);
        nav.navigate(root.clone());
        assert!(Arc::ptr_eq(&parent_rows, &nav.cached_rows(&root)));
        nav.begin(child.clone(), 9);
        nav.batch(crate::directory::Batch {
            path: child.clone(),
            generation: 9,
            entries: vec![file("/home/me/a/new")],
            done: true,
            error: None,
        });
        assert!(Arc::ptr_eq(&parent_rows, &nav.cached_rows(&root)));
        assert!(!Arc::ptr_eq(&child_rows, &nav.cached_rows(&child)));
        nav.refresh();
        assert_eq!(
            nav.cached_rows(&root)[0].message.as_deref(),
            Some("Loading…")
        );
        for index in 0..10 {
            let path = PathBuf::from(format!("/cached/{index}"));
            nav.receive(path.clone(), Ok(vec![]));
            nav.cached_rows(&path);
        }
        assert_eq!(nav.row_cache.len(), 4);
        assert!(
            nav.row_cache
                .iter()
                .all(|cache| cache.folder.starts_with("/cached"))
        );
    }

    #[test]
    fn live_refresh_retains_rows_until_replacement_and_keeps_view_state() {
        let mut nav = nav();
        let root = PathBuf::from("/home/me");
        nav.selections.insert(root.clone(), root.join("b"));
        nav.vertical.insert((root.clone(), false), 91.);
        let previous: Vec<_> = nav.rows(&root).into_iter().map(|row| row.path).collect();
        nav.begin(root.clone(), 7);
        assert_eq!(
            nav.rows(&root)
                .into_iter()
                .map(|row| row.path)
                .collect::<Vec<_>>(),
            previous
        );
        nav.batch(crate::directory::Batch {
            path: root.clone(),
            generation: 7,
            entries: vec![file("/home/me/b")],
            done: true,
            error: None,
        });
        assert_eq!(nav.rows(&root).len(), 1);
        assert_eq!(nav.selection(), Some(root.join("b")));
        assert_eq!(nav.vertical[&(root, false)], 91.);
    }

    #[test]
    fn document_selection_does_not_move_or_expand_directory() {
        let mut nav = nav();
        nav.leave_for_editor();
        nav.reveal_document(Path::new("/home/me/a/deep/notes.txt"));
        assert_eq!(nav.current, Path::new("/home/me"));
        assert_eq!(nav.requested_folders(), vec![PathBuf::from("/home/me")]);
        nav.reveal_document(Path::new("/home/me/readme.txt"));
        assert_eq!(nav.selection(), Some("/home/me/readme.txt".into()));
    }

    #[test]
    fn streamed_generations_and_cache_bound_preserve_selection() {
        let mut nav = nav();
        nav.begin("/home/me".into(), 5);
        nav.batch(crate::directory::Batch {
            path: "/home/me".into(),
            generation: 4,
            entries: vec![file("/home/me/stale")],
            done: true,
            error: None,
        });
        nav.batch(crate::directory::Batch {
            path: "/home/me".into(),
            generation: 5,
            entries: vec![file("/home/me/kept")],
            done: false,
            error: None,
        });
        nav.selections
            .insert("/home/me".into(), "/home/me/kept".into());
        nav.batch(crate::directory::Batch {
            path: "/home/me".into(),
            generation: 5,
            entries: vec![folder("/home/me/a")],
            done: true,
            error: None,
        });
        assert_eq!(nav.selection(), Some("/home/me/kept".into()));
        assert!(
            !nav.rows(Path::new("/home/me"))
                .iter()
                .any(|r| r.path == Path::new("/home/me/stale"))
        );
        for i in 0..100 {
            nav.receive(
                format!("/cached/{i}").into(),
                Ok(vec![file(&format!("/cached/{i}/file"))]),
            );
        }
        assert!(nav.folders.len() <= 64);
        assert!(nav.folders.contains_key(Path::new("/home/me")));
    }
    #[test]
    fn settled_startup_does_not_busy_repaint() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        nav.leave_for_editor();
        let mut delay = std::time::Duration::ZERO;
        for i in 0..20 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(340.0, 440.0),
                    )),
                    time: Some(i as f64),
                    ..Default::default()
                },
                |ui| {
                    nav.show(ui, true);
                },
            );
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            output.textures_delta.clear();
        }
        assert!(
            delay > std::time::Duration::from_millis(100),
            "idle tree requested immediate repaint: {delay:?}"
        );
    }
    #[test]
    #[ignore = "release CPU benchmark; no physical presentation measurement"]
    fn benchmark_ten_thousand_rows() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::new("/fixture".into(), "/fixture".into());
        nav.receive(
            "/fixture".into(),
            Ok((0..10_000)
                .map(|i| file(&format!("/fixture/item-{i:05}.txt")))
                .collect()),
        );
        let mut samples = Vec::new();
        for i in 0..240 {
            let start = std::time::Instant::now();
            nav.cancel_inactive();
            frame(
                &ctx,
                &mut nav,
                vec![
                    egui::Event::PointerMoved(egui::pos2(180., 200.)),
                    wheel(egui::vec2(0.0, -28.0), egui::MouseWheelSource::Wheel),
                ],
                i as f64 / 120.0,
            );
            if i >= 10 {
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "10k rows, 230 synthetic scroll UI CPU frames: p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms; budget 8.33ms; excludes render/present",
            samples[115], samples[218], samples[227], samples[229]
        );
    }
    #[test]
    #[ignore = "release CPU benchmark; excludes filesystem, renderer and presentation"]
    fn benchmark_cached_folder_switches() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::new("/fixture".into(), "/fixture".into());
        for folder in ["/fixture/a", "/fixture/b"] {
            nav.receive(
                folder.into(),
                Ok((0..10_000)
                    .map(|i| file(&format!("{folder}/item-{i:05}.txt")))
                    .collect()),
            );
        }
        let mut samples = Vec::new();
        for i in 0..240 {
            let start = std::time::Instant::now();
            nav.navigate(
                if i % 2 == 0 {
                    "/fixture/a"
                } else {
                    "/fixture/b"
                }
                .into(),
            );
            nav.cancel_inactive();
            frame(&ctx, &mut nav, vec![], i as f64 / 120.);
            if i >= 10 {
                samples.push(start.elapsed().as_secs_f64() * 1000.);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "10k-row cached folder switches, 230 UI CPU frames: p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms; excludes filesystem/render/present",
            samples[115], samples[218], samples[227], samples[229]
        );
    }
    fn folder(path: &str) -> Entry {
        Entry {
            path: path.into(),
            directory: true,
            symlink: false,
        }
    }
    fn file(path: &str) -> Entry {
        Entry {
            path: path.into(),
            directory: false,
            symlink: false,
        }
    }
    fn nav() -> Navigator {
        let mut nav = Navigator::new("/home/me".into(), "/home/me".into());
        nav.receive(
            "/home/me".into(),
            Ok(vec![
                folder("/home/me/a"),
                folder("/home/me/b"),
                file("/home/me/readme"),
            ]),
        );
        nav.receive(
            "/home/me/a".into(),
            Ok(vec![folder("/home/me/a/deep"), file("/home/me/a/notes")]),
        );
        nav.receive("/home/me/a/deep".into(), Ok(vec![]));
        nav.receive("/home/me/b".into(), Ok(vec![file("/home/me/b/other")]));
        nav
    }
    fn frame(
        ctx: &egui::Context,
        nav: &mut Navigator,
        events: Vec<egui::Event>,
        time: f64,
    ) -> NavigationAction {
        let mut action = NavigationAction::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(340.0, 440.0),
                )),
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                action = nav.show(ui, true);
            },
        );
        output.textures_delta.clear();
        action
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn wheel(delta: Vec2, source: egui::MouseWheelSource) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            source,
            modifiers: egui::Modifiers::NONE,
        }
    }
    #[test]
    fn startup_is_one_lazy_directory_and_parent_reaches_root() {
        let mut nav = Navigator::new("/home/me".into(), "/home/me".into());
        assert_eq!(nav.take_requests(), [PathBuf::from("/home/me")]);
        assert_eq!(nav.columns, [PathBuf::from("/home/me")]);
        nav.up();
        nav.up();
        nav.up();
        assert_eq!(nav.current, Path::new("/"));
        assert_eq!(nav.columns, ["/", "/home", "/home/me"].map(PathBuf::from));
        nav.forward();
        assert_eq!(nav.current, Path::new("/home"));
    }
    #[test]
    fn deep_navigation_only_requests_the_displayed_folder() {
        let mut nav = Navigator::new("/fixture".into(), "/fixture".into());
        nav.take_requests();
        nav.navigate("/fixture/one/two/three".into());
        assert_eq!(
            nav.take_requests(),
            [PathBuf::from("/fixture/one/two/three")]
        );
    }
    #[test]
    fn siblings_replace_retained_path_but_keep_cached_folders() {
        let mut nav = nav();
        nav.navigate("/home/me/a".into());
        nav.navigate("/home/me/a/deep".into());
        nav.navigate("/home/me/b".into());
        assert_eq!(nav.columns, ["/home/me", "/home/me/b"].map(PathBuf::from));
        assert!(nav.folders.contains_key(Path::new("/home/me/a/deep")));
        assert_eq!(
            nav.selections[Path::new("/home/me")],
            Path::new("/home/me/b")
        );
        nav.up();
        nav.forward();
        assert_eq!(nav.current, Path::new("/home/me/b"));
        nav.refresh();
        assert!(matches!(
            nav.folders[Path::new("/home/me/b")],
            Listing::Loading
        ));
        assert!(matches!(
            nav.folders[Path::new("/home/me/a")],
            Listing::Ready(_)
        ));
    }
    #[test]
    fn editing_and_refresh_preserve_single_list_location_and_selection() {
        let mut nav = nav();
        nav.navigate("/home/me/a".into());
        nav.select_file(Path::new("/home/me/a/notes"));
        let path = nav.columns.clone();
        let selections = nav.selections.clone();
        assert!(!nav.leave_for_editor());
        nav.refresh();
        assert_eq!(nav.current, Path::new("/home/me/a"));
        assert_eq!(nav.columns, path);
        assert_eq!(nav.selections, selections);
        assert_eq!(nav.requested_folders(), vec![PathBuf::from("/home/me/a")]);
    }

    #[test]
    fn loading_error_empty_and_symlink_rows_are_explicit() {
        let mut nav = Navigator::new("/home/me".into(), "/home/me".into());
        assert_eq!(
            nav.rows(Path::new("/home/me"))[0].message.as_deref(),
            Some("Loading…")
        );
        nav.receive("/home/me".into(), Err("Permission denied".into()));
        assert!(
            nav.rows(Path::new("/home/me"))[0]
                .message
                .as_deref()
                .unwrap()
                .contains("Permission denied")
        );
        nav.receive("/home/me".into(), Ok(vec![]));
        assert_eq!(
            nav.rows(Path::new("/home/me"))[0].message.as_deref(),
            Some("Empty folder")
        );
        let mut link = file("/home/me/link");
        link.symlink = true;
        nav.receive("/home/me".into(), Ok(vec![link]));
        let rows = nav.rows(Path::new("/home/me"));
        assert!(rows[0].symlink && !rows[0].directory);
    }
    #[test]
    fn keyboard_selects_enters_returns_and_opens_file_deliberately() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowDown)], 0.0);
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowRight)], 0.1);
        assert_eq!(nav.current, Path::new("/home/me/a"));
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowDown)], 0.2);
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowDown)], 0.3);
        let action = frame(&ctx, &mut nav, vec![key(egui::Key::Enter)], 0.4);
        assert_eq!(action.open, Some((PathBuf::from("/home/me/a/notes"), true)));
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowLeft)], 0.5);
        assert_eq!(nav.current, Path::new("/home/me"));
        frame(&ctx, &mut nav, vec![key(egui::Key::ArrowRight)], 0.6);
        assert_eq!(nav.current, Path::new("/home/me/a"));
    }
    #[test]
    fn horizontal_gestures_emit_one_history_step_and_ignore_momentum() {
        let mut nav = nav();
        nav.gesture(egui::vec2(80., 2.), egui::TouchPhase::Start, 1.);
        assert_eq!(nav.gesture_step.take(), Some(true));
        for time in [1.05, 1.5, 2.0] {
            nav.gesture(egui::vec2(400., 0.), egui::TouchPhase::Move, time);
            assert_eq!(nav.gesture_step.take(), None);
        }
        nav.gesture(egui::vec2(200., 0.), egui::TouchPhase::End, 2.1);
        nav.gesture(egui::vec2(200., 0.), egui::TouchPhase::Move, 2.8);
        assert_eq!(nav.gesture_step.take(), None);
        nav.gesture(egui::vec2(-90., 0.), egui::TouchPhase::Start, 3.);
        assert_eq!(nav.gesture_step.take(), Some(false));
        assert_eq!(nav.current, Path::new("/home/me"));
    }

    #[test]
    fn unphased_wheel_bursts_are_latched_until_idle() {
        let mut nav = nav();
        nav.gesture(egui::vec2(80., 0.), egui::TouchPhase::Move, 1.);
        assert_eq!(nav.gesture_step.take(), Some(true));
        nav.gesture(egui::vec2(-800., 0.), egui::TouchPhase::Move, 1.1);
        assert_eq!(nav.gesture_step.take(), None);
        nav.gesture(egui::vec2(-80., 0.), egui::TouchPhase::Move, 1.5);
        assert_eq!(nav.gesture_step.take(), Some(false));
    }

    #[test]
    fn native_momentum_start_cannot_emit_a_second_directory_command() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        let event = |source, phase| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(120., 0.),
            phase,
            source,
            modifiers: egui::Modifiers::NONE,
        };
        let first = frame(
            &ctx,
            &mut nav,
            vec![
                egui::Event::PointerMoved(egui::pos2(180., 200.)),
                event(egui::MouseWheelSource::Trackpad, egui::TouchPhase::Start),
            ],
            1.,
        );
        assert_eq!(first.back, Some(true));
        for (index, phase) in [
            egui::TouchPhase::Start,
            egui::TouchPhase::Move,
            egui::TouchPhase::End,
        ]
        .into_iter()
        .enumerate()
        {
            let momentum = frame(
                &ctx,
                &mut nav,
                vec![event(egui::MouseWheelSource::Momentum, phase)],
                2. + index as f64,
            );
            assert_eq!(momentum.back, None);
        }
        let next = frame(
            &ctx,
            &mut nav,
            vec![event(
                egui::MouseWheelSource::Trackpad,
                egui::TouchPhase::Start,
            )],
            5.,
        );
        assert_eq!(next.back, Some(true));
    }

    #[test]
    fn vertical_diagonal_and_small_horizontal_input_do_not_change_location() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        nav.navigate("/home/me/a".into());
        frame(
            &ctx,
            &mut nav,
            vec![egui::Event::PointerMoved(egui::pos2(180.0, 200.0))],
            0.0,
        );
        for (index, delta) in [
            egui::vec2(0.0, 120.0),
            egui::vec2(80.0, 90.0),
            egui::vec2(15.0, 0.0),
        ]
        .into_iter()
        .enumerate()
        {
            frame(
                &ctx,
                &mut nav,
                vec![wheel(delta, egui::MouseWheelSource::Trackpad)],
                index as f64 + 1.0,
            );
            assert_eq!(nav.current, Path::new("/home/me/a"));
        }
    }
    #[test]
    fn pointer_enters_folder_and_opens_file_in_single_list() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        frame(&ctx, &mut nav, vec![], 0.0);
        fn click(
            ctx: &egui::Context,
            nav: &mut Navigator,
            pos: egui::Pos2,
            time: f64,
        ) -> NavigationAction {
            frame(
                ctx,
                nav,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                time,
            );
            frame(
                ctx,
                nav,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                time + 0.01,
            )
        }
        click(&ctx, &mut nav, egui::pos2(65.0, 20.0), 0.1);
        assert_eq!(nav.current, Path::new("/home/me/a"));
        frame(&ctx, &mut nav, vec![], 0.2);
        let action = click(&ctx, &mut nav, egui::pos2(100.0, 48.0), 0.3);
        assert_eq!(
            action.open,
            Some((PathBuf::from("/home/me/a/notes"), false))
        );
        assert!(nav.focused);
    }
    #[test]
    fn vertical_scroll_survives_editing_and_parent_navigation() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        let entries = (0..100)
            .map(|i| file(&format!("/home/me/a/item{i:03}")))
            .collect();
        nav.receive("/home/me/a".into(), Ok(entries));
        nav.navigate("/home/me/a".into());
        frame(
            &ctx,
            &mut nav,
            vec![egui::Event::PointerMoved(egui::pos2(245.0, 200.0))],
            0.0,
        );
        frame(
            &ctx,
            &mut nav,
            vec![wheel(
                egui::vec2(0.0, -160.0),
                egui::MouseWheelSource::Wheel,
            )],
            0.1,
        );
        for i in 0..60 {
            frame(&ctx, &mut nav, vec![], 0.2 + i as f64 / 60.0);
        }
        let child_key = (PathBuf::from("/home/me/a"), false);
        let parent_key = (PathBuf::from("/home/me"), false);
        let offset = nav.vertical[&child_key];
        assert!(offset > 20.0);
        assert_eq!(nav.vertical.get(&parent_key).copied().unwrap_or(0.), 0.0);
        nav.leave_for_editor();
        frame(&ctx, &mut nav, vec![], 2.0);
        nav.resume();
        frame(&ctx, &mut nav, vec![], 2.1);
        assert_eq!(nav.vertical[&child_key], offset);
        nav.up();
        nav.up();
        nav.forward();
        nav.forward();
        frame(&ctx, &mut nav, vec![], 2.2);
        assert_eq!(nav.vertical[&child_key], offset);
    }
    #[test]
    fn single_list_never_exposes_retained_ancestor_rows() {
        let ctx = egui::Context::default();
        let mut nav = nav();
        nav.navigate("/home/me/a".into());
        frame(&ctx, &mut nav, vec![], 0.);
        assert!(
            !nav.vertical
                .contains_key(&(PathBuf::from("/home/me"), false))
        );
        assert!(
            nav.vertical
                .contains_key(&(PathBuf::from("/home/me/a"), false))
        );
    }
}
