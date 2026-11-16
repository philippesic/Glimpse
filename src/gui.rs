use glimpse::directory::Entry;
use glimpse::{
    cli, directory,
    document::DiskFile,
    io::{IoWorker, Job, Reply, SaveDestination},
    lifecycle, observe, paths, search, storage, wake,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Root,
    input::{Input, InputEvent, InputState, Position},
    text::TextView,
};
use std::{collections::HashMap, path::PathBuf};

pub fn run(request: cli::Request) {
    Application::new()
        .with_assets(gpui_component_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            #[cfg(target_os = "linux")]
            cx.bind_keys([
                KeyBinding::new(
                    "ctrl-home",
                    gpui_component::input::MoveToStart,
                    Some("Input"),
                ),
                KeyBinding::new("ctrl-end", gpui_component::input::MoveToEnd, Some("Input")),
                KeyBinding::new(
                    "ctrl-shift-home",
                    gpui_component::input::SelectToStart,
                    Some("Input"),
                ),
                KeyBinding::new(
                    "ctrl-shift-end",
                    gpui_component::input::SelectToEnd,
                    Some("Input"),
                ),
            ]);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            let bounds = Bounds::centered(None, size(px(1280.), px(844.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx| Shell::new(request, window, cx));
                    let weak = view.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        weak.update(cx, |shell, cx| {
                            shell.close(true, window, cx);
                            false
                        })
                        .unwrap_or(true)
                    });
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Cannot open GPUI window");
            cx.activate(true);
        });
}
struct Document {
    id: u64,
    disk: DiskFile,
    input: Entity<InputState>,
    saved: String,
    generation: u64,
    preview: bool,
    saving: bool,
    restored: bool,
    external: Option<(DiskFile, String)>,
}
impl Document {
    fn dirty(&self, cx: &App) -> bool {
        self.restored || self.input.read(cx).value().as_str() != self.saved
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Path,
    Filter,
    Files,
    Text,
    Palette,
    Settings,
    Recovery,
    Line,
    NewFile,
    NewFolder,
    Rename,
}
struct CloseDecision {
    quit: bool,
    documents: Vec<u64>,
    saving: bool,
}
struct Shell {
    request: cli::Request,
    io: IoWorker,
    directories: directory::Directories,
    observer: observe::Observer,
    search: search::Search,
    syntax: glimpse::syntax::Syntax,
    search_id: u64,
    hits: Vec<search::Hit>,
    scope: Option<PathBuf>,
    current: PathBuf,
    history: Vec<PathBuf>,
    future: Vec<PathBuf>,
    nav_focus: FocusHandle,
    scrolls: HashMap<PathBuf, UniformListScrollHandle>,
    selection: Option<PathBuf>,
    selections: HashMap<PathBuf, PathBuf>,
    entries: Vec<Entry>,
    listing_id: u64,
    listing_done: bool,
    filter: String,
    documents: Vec<Document>,
    active: Option<u64>,
    next_id: u64,
    open_token: u64,
    opening: HashMap<u64, (bool, usize)>,
    hit_versions: HashMap<u64, u64>,
    panel: Option<Panel>,
    query: Entity<InputState>,
    reading: bool,
    status: String,
    settings: storage::Settings,
    subscriptions: Vec<Subscription>,
    close_pending: Option<CloseDecision>,
    closings: glimpse::closing::Closings,
    quit_after_cleanup: bool,
    destination: Option<SaveDestination>,
    destination_busy: bool,
    records: Vec<storage::Record>,
    ready: bool,
    startup_failed: bool,
    requested: Vec<(u64, PathBuf)>,
    finished: bool,
    finish_pending: usize,
}
impl Shell {
    fn new(request: cli::Request, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::bounded(1);
        let wake = wake::Context::new(move || {
            let _ = tx.try_send(());
        });
        let io = IoWorker::new(wake.clone());
        let directories = directory::Directories::new(wake.clone());
        let observer = observe::Observer::new(wake.clone());
        let syntax = glimpse::syntax::Syntax::new(wake.clone());
        let search = search::Search::new(wake);
        let current = request
            .folder
            .clone()
            .or_else(|| {
                request
                    .files
                    .first()
                    .and_then(|p| p.parent().map(|p| p.to_owned()))
            })
            .unwrap_or_else(|| std::env::home_dir().unwrap());
        let query = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Enter a path");
            input.set_cursor_blink(false, cx);
            input
        });
        let subscription = cx.subscribe_in(&query, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.submit(window, cx);
            }
            if matches!(event, InputEvent::Change) && this.panel == Some(Panel::Filter) {
                this.filter = this.query.read(cx).value().to_string();
                cx.notify();
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while rx.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| this.receive(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut this = Self {
            scope: request.scope.clone(),
            request,
            io,
            directories,
            observer,
            search,
            syntax,
            current,
            history: vec![],
            future: vec![],
            nav_focus: cx.focus_handle(),
            scrolls: HashMap::new(),
            selection: None,
            selections: HashMap::new(),
            entries: vec![],
            listing_id: 0,
            listing_done: false,
            filter: String::new(),
            documents: vec![],
            active: None,
            next_id: 1,
            open_token: 0,
            opening: HashMap::new(),
            hit_versions: HashMap::new(),
            panel: None,
            query,
            reading: false,
            status: String::new(),
            settings: storage::Settings::default(),
            subscriptions: vec![subscription],
            close_pending: None,
            closings: glimpse::closing::Closings::default(),
            quit_after_cleanup: false,
            destination: None,
            destination_busy: false,
            records: vec![],
            ready: false,
            startup_failed: false,
            requested: vec![],
            finished: false,
            finish_pending: 0,
            search_id: 0,
            hits: vec![],
        };
        window.focus(&this.nav_focus);
        this.refresh();
        this.io.send(Job::LoadSettings);
        let files = this.request.files.clone();
        for path in files {
            this.open(path, false, this.request.line);
        }
        this
    }
    fn watch(&mut self) {
        let mut parents = std::collections::HashSet::from([self.current.clone()]);
        for doc in &self.documents {
            if let Some(p) = doc.disk.path.parent() {
                parents.insert(p.to_owned());
            }
        }
        self.observer.update(parents);
    }
    fn refresh(&mut self) {
        self.entries.clear();
        self.listing_done = false;
        self.listing_id = self.directories.request(self.current.clone()).unwrap_or(0);
        self.watch();
    }
    fn navigate(&mut self, path: PathBuf) {
        if path == self.current {
            return;
        }
        if let Some(selection) = self.selection.take() {
            self.selections.insert(self.current.clone(), selection);
        }
        self.history
            .push(std::mem::replace(&mut self.current, path));
        self.future.clear();
        self.selection = self.selections.get(&self.current).cloned();
        self.filter.clear();
        self.hits.clear();
        self.search_id = self.search.cancel();
        self.refresh();
    }
    fn open(&mut self, path: PathBuf, preview: bool, line: usize) {
        if self.destination_busy {
            self.status = "Finish the destination save before opening another file".into();
            return;
        }
        if path.is_dir() {
            self.navigate(path);
            return;
        }
        self.open_token += 1;
        self.opening.insert(self.open_token, (preview, line));
        self.io.send(Job::Open(path, self.open_token, false));
    }
    fn add_document(
        &mut self,
        disk: DiskFile,
        text: String,
        preview: bool,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(doc) = self.documents.iter_mut().find(|d| d.disk.same_file(&disk)) {
            if !preview {
                doc.preview = false;
            }
            if self.request.files.contains(&disk.path)
                && !self.requested.iter().any(|(_, path)| *path == disk.path)
            {
                self.requested.push((doc.id, disk.path.clone()));
            }
            self.active = Some(doc.id);
            self.focus(window, cx);
            return;
        }
        if preview {
            self.documents.retain(|d| !d.preview || d.dirty(cx));
        }
        let id = self.next_id;
        self.next_id += 1;
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .code_editor("text")
                .line_number(self.settings.line_numbers)
                .soft_wrap(self.settings.wrap)
                .default_value(text.clone());
            input.set_cursor_blink(false, cx);
            input
        });
        self.subscriptions.push(
            cx.subscribe_in(&input, window, move |this, _, event, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                    doc.generation += 1;
                    glimpse::metrics::event("change", id, doc.generation);
                    doc.preview = false;
                    if !this.request.pick {
                        this.io.send(Job::Recover(
                            id,
                            storage::Draft {
                                path: doc.disk.path.clone(),
                                target: Some(doc.disk.target.clone()),
                                text: doc.input.read(cx).value().to_string(),
                                base: doc.disk.bytes.clone(),
                                generation: doc.generation,
                            },
                        ));
                    }
                }
                if let Some(doc) = this.documents.iter().find(|d| d.id == id) {
                    this.syntax.request(
                        id,
                        doc.generation,
                        &doc.input.read(cx).value(),
                        &doc.disk.path,
                        true,
                    );
                }
                cx.notify();
            }),
        );
        if self.request.files.contains(&disk.path) {
            self.requested.push((id, disk.path.clone()));
        }
        self.documents.push(Document {
            id,
            disk,
            input: input.clone(),
            saved: text,
            generation: 0,
            preview,
            saving: false,
            restored: false,
            external: None,
        });
        self.syntax.request(
            id,
            0,
            &input.read(cx).value(),
            &disk_path(&self.documents, id),
            true,
        );
        self.active = Some(id);
        self.reading = false;
        self.watch();
        let text = input.read(cx).value();
        let (position, clamped) = glimpse::editor::position(&text, line, self.request.column);
        let byte = text
            .char_indices()
            .nth(position)
            .map_or(text.len(), |(byte, _)| byte);
        let prefix = &text[..byte];
        let row = prefix.bytes().filter(|&byte| byte == b'\n').count() as u32;
        let column = prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count() as u32;
        input.update(cx, |input, cx| {
            input.set_cursor_position(Position::new(row, column), window, cx)
        });
        if clamped {
            self.status = "Requested position clamped to a text boundary".into();
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            self.syntax.request(
                doc.id,
                doc.generation,
                &doc.input.read(cx).value(),
                &doc.disk.path,
                true,
            );
            doc.input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.nav_focus);
        }
    }
    fn panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = Some(panel);
        let value = if panel == Panel::Path {
            paths::display(&self.current)
        } else {
            String::new()
        };
        self.query.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        if panel == Panel::Recovery {
            self.io.send(Job::Inventory);
        }
        cx.notify();
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.query.read(cx).value().to_string();
        match self.panel {
            Some(Panel::Palette) => {
                let query = value.to_lowercase();
                if let Some(command) = glimpse::commands::COMMANDS
                    .iter()
                    .find(|command| command.label.to_lowercase().contains(&query))
                {
                    self.panel = None;
                    self.command(command.id, window, cx);
                }
            }
            Some(Panel::Path) => {
                let path = PathBuf::from(value);
                if path.exists() {
                    self.open(path, false, 1);
                    self.panel = None;
                } else {
                    self.status = "Path does not exist".into();
                }
            }
            Some(Panel::Files | Panel::Text) => {
                let scope = self.scope.clone().unwrap_or_else(|| self.current.clone());
                self.search_id = self
                    .search
                    .start_filtered(
                        scope,
                        value,
                        self.panel == Some(Panel::Text),
                        search::Filters {
                            ignored: self.settings.search_ignored,
                            metadata: self.settings.search_metadata,
                            exclude: self.settings.search_exclude.clone(),
                            file_bytes: self.settings.search_file_mib * 1024 * 1024,
                        },
                    )
                    .unwrap();
                self.hits.clear();
                self.status = "Searching…".into();
            }
            Some(Panel::NewFile | Panel::NewFolder | Panel::Rename) => {
                let path = self.current.join(&value);
                if value.is_empty() || PathBuf::from(&value).components().count() != 1 {
                    self.status = "Enter one filename".into();
                } else if self.panel == Some(Panel::Rename) {
                    if let Some(source) = self.selection.clone() {
                        if self.documents.iter().any(|doc| doc.disk.path == source) {
                            self.status = "Close the file before renaming".into();
                        } else {
                            self.io.send(Job::Rename(source, path));
                            self.panel = None;
                        }
                    }
                } else {
                    self.io.send(Job::Create(
                        path,
                        self.panel == Some(Panel::NewFolder),
                        String::new(),
                    ));
                    self.panel = None;
                }
            }
            Some(Panel::Line) => {
                if let Ok(line) = value.parse::<u32>() {
                    if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
                        doc.input.update(cx, |input, cx| {
                            input.set_cursor_position(
                                Position::new(line.saturating_sub(1), 0),
                                window,
                                cx,
                            )
                        });
                    }
                    self.panel = None;
                }
            }
            _ => {}
        }
        cx.notify();
    }
    fn receive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Ok(batch) = self.directories.replies.try_recv() {
            if batch.path == self.current && batch.generation == self.listing_id {
                self.entries.extend(batch.entries);
                self.entries.sort_by(|a, b| {
                    b.directory
                        .cmp(&a.directory)
                        .then_with(|| a.path.cmp(&b.path))
                });
                self.listing_done = batch.done;
                if let Some(error) = batch.error {
                    self.startup_failed |= !self.ready;
                    self.status = error;
                }
            }
        }
        while let Ok(reply) = self.io.replies.try_recv() {
            match reply {
                Reply::Open(path, token, result) => {
                    if let Some((preview, line)) = self.opening.remove(&token) {
                        match result {
                            Ok((disk, text)) => {
                                if self.destination_busy {
                                    continue;
                                }
                                if let Some(expected) = self.hit_versions.remove(&token)
                                    && (expected != search::fingerprint(&disk.bytes)
                                        || self
                                            .documents
                                            .iter()
                                            .any(|doc| doc.disk.same_file(&disk) && doc.dirty(cx)))
                                {
                                    self.status = "Search result changed or has unsaved edits. Search again before jumping.".into();
                                    continue;
                                }
                                self.add_document(disk, text, preview, line, window, cx)
                            }
                            Err(e) => {
                                self.startup_failed |=
                                    !self.ready && self.request.files.contains(&path);
                                self.status = e;
                            }
                        }
                    }
                }
                Reply::Cleared(id, generation, result) => {
                    let current = self
                        .documents
                        .iter()
                        .find(|d| d.id == id)
                        .map_or(generation, |d| d.generation);
                    match self.closings.acknowledge(id, generation, current, result) {
                        glimpse::closing::Outcome::Completed { .. } => {
                            if !self.finished
                                && self.requested.iter().any(|(requested, _)| *requested == id)
                            {
                                let _ = lifecycle::notify(b'1');
                                self.finished = true;
                            }
                            self.documents.retain(|d| d.id != id);
                            if self.active == Some(id) {
                                self.active = self.documents.last().map(|d| d.id);
                            }
                            self.focus(window, cx);
                            self.finish_close(cx);
                        }
                        glimpse::closing::Outcome::Changed => {
                            self.quit_after_cleanup = false;
                            self.status = "Close cancelled because the document changed".into();
                        }
                        glimpse::closing::Outcome::Failed(error) => {
                            self.status =
                                format!("Recovery cleanup failed; document kept open: {error}")
                        }
                        glimpse::closing::Outcome::Ignored => {}
                    }
                }
                Reply::Save(id, text, result) => {
                    glimpse::metrics::event("save-reply", id, 0);
                    let succeeded = result.is_ok();
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                        doc.saving = false;
                        match result {
                            Ok(disk) => {
                                doc.disk = disk;
                                doc.saved = text;
                                doc.restored = false;
                                self.status = "Saved".into();
                            }
                            Err(e) => self.status = e,
                        }
                    }
                    if succeeded && self.close_pending.as_ref().is_some_and(|d| d.saving) {
                        self.continue_close(window, cx);
                    }
                }
                Reply::SavedTo(destination, result) => {
                    let succeeded = result.is_ok();
                    self.destination_busy = false;
                    match result {
                        Ok(disk) => {
                            if !destination.copy
                                && let Some(doc) =
                                    self.documents.iter_mut().find(|d| d.id == destination.id)
                            {
                                doc.disk = disk;
                                doc.saved = destination.text;
                                doc.restored = false;
                            }
                            self.status = "Saved".into();
                        }
                        Err(e) => self.status = e,
                    }
                    if succeeded && self.close_pending.as_ref().is_some_and(|d| d.saving) {
                        self.continue_close(window, cx);
                    }
                }
                Reply::Prepared(mut destination, result) => match result {
                    Ok(None) => {
                        destination.expected = None;
                        self.io.send(Job::SaveTo(*destination));
                    }
                    Ok(Some(disk)) => {
                        destination.expected = Some(disk);
                        self.destination = Some(*destination);
                        self.status = "Destination exists. Replace this observed version?".into();
                    }
                    Err(e) => {
                        self.destination_busy = false;
                        self.status = e;
                    }
                },
                Reply::Checked(id, generation, result) => {
                    if let Some(doc) = self
                        .documents
                        .iter_mut()
                        .find(|d| d.id == id && d.generation == generation)
                    {
                        match result {
                            Ok((disk, text)) if disk.target != doc.disk.target => {
                                self.status = format!(
                                    "Destination changed from {} to {}. Local text retained; use Save Copy or explicitly Reload disk.",
                                    paths::display(&doc.disk.target),
                                    paths::display(&disk.target)
                                );
                                doc.external = Some((disk, text));
                            }
                            Ok((disk, text)) if disk.bytes != doc.disk.bytes => {
                                if doc.dirty(cx) {
                                    doc.external = Some((disk, text));
                                    self.status = "Changed on disk. Local edits retained. Reload or Save Copy.".into();
                                } else {
                                    let length = doc.input.read(cx).value().encode_utf16().count();
                                    doc.input.update(cx, |input, cx| {
                                        input.replace_text_in_range(
                                            Some(0..length),
                                            &text,
                                            window,
                                            cx,
                                        )
                                    });
                                    doc.saved = text;
                                    doc.disk = disk;
                                    self.status =
                                        "Reloaded external change (Undo available)".into();
                                }
                            }
                            Ok((disk, _)) => doc.disk = disk,
                            Err(e) => self.status = format!("External check: {e}"),
                        }
                    }
                }
                Reply::SettingsLoaded(result) => match result {
                    Ok(settings) => {
                        self.settings = settings;
                        for doc in &self.documents {
                            doc.input.update(cx, |input, cx| {
                                input.set_soft_wrap(self.settings.wrap, window, cx);
                                input.set_line_number(self.settings.line_numbers, window, cx);
                            });
                        }
                    }
                    Err(e) => self.status = e,
                },
                Reply::Records(result) => match result {
                    Ok(inventory) => {
                        self.records = inventory.records;
                        self.status = format!("{} recovery/history records", self.records.len());
                    }
                    Err(e) => self.status = e,
                },
                Reply::Imported(import_id, result) => match result {
                    Ok((draft, _)) => {
                        let mut disk = DiskFile::draft();
                        disk.path = draft.path.clone();
                        disk.bytes = draft.base.clone();
                        disk.bom = draft.base.starts_with(&[0xef, 0xbb, 0xbf]);
                        let next_id = self.next_id;
                        self.next_id = import_id;
                        self.add_document(disk, draft.text, false, 1, window, cx);
                        self.next_id = self.next_id.max(next_id);
                        if let Some(doc) = self.documents.last_mut() {
                            doc.restored = true;
                            doc.generation = draft.generation + 1;
                        }
                    }
                    Err(e) => self.status = e,
                },
                Reply::Chosen(path, result) => match result {
                    Ok(()) => {
                        use std::{io::Write, os::unix::ffi::OsStrExt};
                        let mut out = std::io::stdout().lock();
                        let result = if self.request.nul {
                            out.write_all(path.as_os_str().as_bytes())
                                .and_then(|_| out.write_all(&[0]))
                        } else {
                            writeln!(
                                out,
                                "{}",
                                cli::selection_json(&path, self.request.directory)
                            )
                        };
                        if result.and_then(|_| out.flush()).is_ok() {
                            self.finished = true;
                            cx.quit();
                        } else {
                            crate::EXIT_STATUS.store(2, std::sync::atomic::Ordering::Relaxed);
                            cx.quit();
                        }
                    }
                    Err(e) => self.status = e,
                },
                Reply::Finished(_, _, result) => {
                    self.finish_pending = self.finish_pending.saturating_sub(1);
                    if let Err(e) = result {
                        self.status = e;
                        self.finish_pending = usize::MAX;
                    }
                    if self.finish_pending == 0 {
                        self.finished = true;
                        let _ = lifecycle::notify(b'0');
                        self.status = "CLI request finished".into();
                    }
                }
                Reply::Done(Err(error)) => self.status = error,
                Reply::Changed(result) => match result {
                    Ok(()) => self.refresh(),
                    Err(e) => self.status = e,
                },
                _ => {}
            }
        }
        while let Ok(result) = self.search.replies.try_recv() {
            if result.generation == self.search_id {
                self.hits = result.hits;
                self.status = result.state;
            }
        }
        if let Some(result) = self.observer.changed() {
            match result {
                Ok(()) => {
                    for doc in &self.documents {
                        if !doc.disk.target.as_os_str().is_empty() {
                            self.io
                                .send(Job::Check(doc.id, doc.disk.path.clone(), doc.generation));
                        }
                    }
                    self.refresh();
                }
                Err(e) => self.status = e,
            }
        }
        if let Some(highlight) = self.syntax.take()
            && let Some(doc) = self
                .documents
                .iter()
                .find(|d| d.id == highlight.id && d.generation == highlight.generation)
        {
            let text = doc.input.read(cx).value();
            let spans = highlight
                .spans
                .into_iter()
                .map(|span| {
                    let start = glimpse::editor::utf16_to_byte(&text, span.start);
                    let end = glimpse::editor::utf16_to_byte(&text, span.start + span.length);
                    let color = glimpse::syntax::readable_rgb(span.rgb, [24, 29, 37]);
                    (
                        start..end,
                        HighlightStyle {
                            color: Some(
                                rgb(((color[0] as u32) << 16)
                                    | ((color[1] as u32) << 8)
                                    | color[2] as u32)
                                .into(),
                            ),
                            font_style: span.italic.then_some(FontStyle::Italic),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            doc.input
                .update(cx, |input, cx| input.set_external_highlights(spans, cx));
            self.status = highlight.state;
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        glimpse::metrics::event("save-command", self.active.unwrap_or(0), 0);
        if self.request.pick {
            return;
        }
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| Some(d.id) == self.active)
        {
            if doc.saving {
                return;
            }
            if doc.disk.target.as_os_str().is_empty() {
                self.save_as(false, window, cx);
                return;
            }
            if doc.external.is_some() {
                self.status =
                    "Resolve the external change before saving. Save Copy preserves local edits."
                        .into();
                return;
            }
            glimpse::metrics::event("save-queued", doc.id, doc.generation);
            doc.saving = true;
            self.io.send(Job::Save {
                id: doc.id,
                disk: doc.disk.clone(),
                text: doc.input.read(cx).value().to_string(),
                generation: doc.generation,
                without_recovery: false,
            });
        }
    }
    fn save_as(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.pick || self.destination_busy {
            return;
        }
        let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) else {
            return;
        };
        if !copy && !self.finished && self.requested.iter().any(|(id, _)| *id == doc.id) {
            self.status = "Finish or Abort the CLI request before rebinding this file".into();
            return;
        }
        let mut destination = SaveDestination {
            id: doc.id,
            path: PathBuf::new(),
            text: doc.input.read(cx).value().to_string(),
            copy,
            bom: doc.disk.bom,
            generation: doc.generation,
            expected: None,
            original: doc.disk.clone(),
            without_recovery: false,
        };
        let prompt = cx.prompt_for_new_path(&self.current, Some(&paths::name(&doc.disk.path)));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    if this.documents.iter().any(|doc| {
                        doc.id != destination.id
                            && (doc.disk.path == path
                                || std::fs::canonicalize(&path).ok().as_ref()
                                    == Some(&doc.disk.target))
                    }) {
                        this.status = "Close the destination's open tab before replacing it".into();
                    } else {
                        destination.path = path;
                        this.destination_busy = true;
                        this.io.send(Job::Prepare(destination));
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn close(&mut self, quit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.io.pending_mutations() > 0 || self.destination_busy || !self.closings.is_empty() {
            self.status = "Wait for pending writes or recovery cleanup".into();
            cx.notify();
            return;
        }
        let documents = self
            .documents
            .iter()
            .filter(|d| quit || Some(d.id) == self.active)
            .map(|d| d.id)
            .collect();
        self.close_pending = Some(CloseDecision {
            quit,
            documents,
            saving: false,
        });
        self.continue_close(window, cx);
    }
    fn continue_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(decision) = &self.close_pending else {
            return;
        };
        let dirty = self
            .documents
            .iter()
            .find(|d| decision.documents.contains(&d.id) && d.dirty(cx))
            .map(|d| d.id);
        if let Some(id) = dirty {
            if decision.saving {
                self.active = Some(id);
                self.save(window, cx);
            }
        } else {
            self.commit_close(false, cx);
        }
        cx.notify();
    }
    fn commit_close(&mut self, discard: bool, cx: &mut Context<Self>) {
        let Some(decision) = self.close_pending.take() else {
            return;
        };
        self.quit_after_cleanup = decision.quit;
        for doc in &self.documents {
            if decision.documents.contains(&doc.id)
                && self.closings.begin(doc.id, doc.generation, discard)
            {
                self.io.send(Job::ClearThrough(doc.id, doc.generation));
            }
        }
        self.finish_close(cx);
    }
    fn finish_close(&mut self, cx: &mut Context<Self>) {
        if self.quit_after_cleanup && self.closings.is_empty() {
            if !self.documents.is_empty() {
                self.quit_after_cleanup = false;
                self.status = "Quit cancelled because another document is open".into();
                return;
            }
            if !self.finished && (self.request.wait || self.request.pick) {
                let _ = lifecycle::notify(b'1');
                crate::EXIT_STATUS.store(1, std::sync::atomic::Ordering::Relaxed);
            }
            cx.quit();
        }
    }
    fn command(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.pick
            && matches!(
                command,
                "named-file"
                    | "new-folder"
                    | "rename"
                    | "recovery"
                    | "settings"
                    | "save-settings"
                    | "wrap"
                    | "numbers"
            )
        {
            self.status = "This picker is read-only".into();
            cx.notify();
            return;
        }
        match command {
            "cancel" if self.request.pick => {
                crate::EXIT_STATUS.store(1, std::sync::atomic::Ordering::Relaxed);
                cx.quit();
            }
            "named-file" => self.panel(Panel::NewFile, window, cx),
            "new-folder" => self.panel(Panel::NewFolder, window, cx),
            "rename" => self.panel(Panel::Rename, window, cx),
            "navigator" => window.focus(&self.nav_focus),
            "find" => {
                self.focus(window, cx);
                window.dispatch_action(Box::new(gpui_component::input::Search), cx);
            }
            "submit" => self.submit(window, cx),
            "dismiss" => {
                self.panel = None;
                self.hits.clear();
                self.focus(window, cx);
            }
            "cancel-close" => {
                self.close_pending = None;
                for id in self.closings.cancel() {
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                        doc.generation += 1;
                        if doc.dirty(cx) {
                            self.io.send(Job::Recover(
                                id,
                                storage::Draft {
                                    path: doc.disk.path.clone(),
                                    target: Some(doc.disk.target.clone()),
                                    text: doc.input.read(cx).value().to_string(),
                                    base: doc.disk.bytes.clone(),
                                    generation: doc.generation,
                                },
                            ));
                        }
                    }
                }
                self.quit_after_cleanup = false;
                self.focus(window, cx);
            }
            "replace-destination" => {
                if let Some(destination) = self.destination.take() {
                    self.io.send(Job::SaveTo(destination));
                }
            }
            "cancel-destination" => {
                self.destination = None;
                self.destination_busy = false;
            }
            "save-settings" => self.io.send(Job::Settings(self.settings.clone())),
            "wrap" | "numbers" => {
                if command == "wrap" {
                    self.settings.wrap = !self.settings.wrap;
                } else {
                    self.settings.line_numbers = !self.settings.line_numbers;
                }
                for doc in &self.documents {
                    doc.input.update(cx, |input, cx| {
                        input.set_soft_wrap(self.settings.wrap, window, cx);
                        input.set_line_number(self.settings.line_numbers, window, cx);
                    });
                }
            }

            "save" => self.save(window, cx),
            "save-close" => {
                if let Some(decision) = self.close_pending.as_mut() {
                    decision.saving = true;
                }
                self.continue_close(window, cx);
            }
            "retry-close" => {
                for (id, generation) in self.closings.retry() {
                    self.io.send(Job::ClearThrough(id, generation));
                }
            }
            "save-as" => self.save_as(false, window, cx),
            "save-copy" => self.save_as(true, window, cx),
            "new" if !self.request.pick => {
                self.add_document(DiskFile::draft(), String::new(), false, 1, window, cx)
            }
            "open" | "folder" => {
                let prompt = cx.prompt_for_paths(PathPromptOptions {
                    files: command == "open",
                    directories: true,
                    multiple: command == "open",
                    prompt: None,
                });
                cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = prompt.await {
                        let _ = this.update(cx, |this, cx| {
                            for path in paths {
                                this.open(path, false, 1);
                            }
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            "path" => self.panel(Panel::Path, window, cx),
            "filter" => self.panel(Panel::Filter, window, cx),
            "find-files" => self.panel(Panel::Files, window, cx),
            "find-text" => self.panel(Panel::Text, window, cx),
            "commands" => self.panel(Panel::Palette, window, cx),
            "settings" => self.panel(Panel::Settings, window, cx),
            "recovery" => self.panel(Panel::Recovery, window, cx),
            "line" => self.panel(Panel::Line, window, cx),
            "parent" => {
                if let Some(path) = self.current.parent().map(|p| p.to_owned()) {
                    self.navigate(path);
                }
            }
            "home" => self.navigate(std::env::home_dir().unwrap()),
            "back" => {
                if let Some(path) = self.history.pop() {
                    self.future.push(std::mem::replace(&mut self.current, path));
                    self.refresh();
                }
            }
            "forward" => {
                if let Some(path) = self.future.pop() {
                    self.history
                        .push(std::mem::replace(&mut self.current, path));
                    self.refresh();
                }
            }
            "refresh" => self.refresh(),
            "read" => self.reading = !self.reading,
            "scope" => {
                self.scope = if self.scope.is_some() {
                    None
                } else {
                    Some(self.current.clone())
                };
                self.search_id = self.search.cancel();
                self.hits.clear();
            }
            "close" => self.close(false, window, cx),
            "quit" => self.close(true, window, cx),
            "abort" => {
                let _ = lifecycle::notify(b'1');
                self.finished = true;
                self.status = "CLI request aborted".into();
            }
            "finish" => {
                if self.requested.iter().any(|(id, _)| {
                    self.documents
                        .iter()
                        .any(|d| d.id == *id && (d.dirty(cx) || d.saving))
                }) {
                    self.status = "Save requested documents before finishing".into();
                } else if !self.finished {
                    self.finish_pending = self.requested.len();
                    for (id, path) in &self.requested {
                        if let Some(doc) = self.documents.iter().find(|d| d.id == *id) {
                            self.io.send(Job::Finish(
                                *id,
                                doc.generation,
                                doc.disk.clone(),
                                vec![path.clone()],
                            ));
                        }
                    }
                }
            }
            "choose" if self.request.pick => {
                let path = if self.request.directory {
                    self.current.clone()
                } else if let Some(path) = &self.selection {
                    path.clone()
                } else {
                    return;
                };
                self.io.send(Job::Choose(path, self.request.directory));
            }
            "trash" if !self.request.pick => {
                if let Some(path) = self.selection.clone() {
                    if self.documents.iter().any(|d| d.disk.path == path) {
                        self.status = "Close this file before moving it to Trash".into();
                    } else {
                        self.io.send(Job::Trash(path));
                    }
                }
            }
            "reload" => {
                if let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|d| Some(d.id) == self.active)
                    && let Some((disk, text)) = doc.external.take()
                {
                    let length = doc.input.read(cx).value().encode_utf16().count();
                    doc.input.update(cx, |input, cx| {
                        input.replace_text_in_range(Some(0..length), &text, window, cx)
                    });
                    doc.disk = disk;
                    doc.saved = text;
                }
            }
            "external" | "finder" => {
                if let Some(path) = self.selection.clone() {
                    let path = if command == "finder" {
                        path.parent().unwrap_or(&path).to_owned()
                    } else {
                        path
                    };
                    #[cfg(target_os = "linux")]
                    let opener = "xdg-open";
                    #[cfg(target_os = "macos")]
                    let opener = "open";
                    if let Err(e) = std::process::Command::new(opener).arg(path).spawn() {
                        self.status = e.to_string();
                    }
                }
            }
            "copy-path" => {
                cx.write_to_clipboard(ClipboardItem::new_string(paths::display(
                    self.selection.as_ref().unwrap_or(&self.current),
                )));
            }
            _ => self.status = format!("Command not yet available in the GPUI port: {command}"),
        }
        cx.notify();
    }
    fn button(
        &self,
        label: impl Into<SharedString>,
        command: &'static str,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(command)
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x303944))
            .cursor_pointer()
            .child(label.into())
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }
}
impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.ready && self.listing_done && self.opening.is_empty() {
            self.ready = true;
            let failed = self.startup_failed;
            window.on_next_frame(move |window, _| {
                window.on_next_frame(move |_, _| {
                    let _ = lifecycle::notify(if failed { b'2' } else { b'0' });
                    if failed {
                        eprintln!("GLIMPSE_STARTUP_FAILED");
                    } else {
                        eprintln!("GLIMPSE_READY");
                    }
                })
            });
            window.request_animation_frame();
        }
        let mut toolbar = div()
            .flex()
            .gap_2()
            .p_2()
            .bg(rgb(0x222933))
            .child(self.button("‹", "back", cx))
            .child(self.button("›", "forward", cx))
            .child(self.button("↑", "parent", cx))
            .child(self.button(paths::display(&self.current), "path", cx))
            .child(self.button("Open", "open", cx))
            .child(self.button("Filter", "filter", cx))
            .child(self.button("Files", "find-files", cx))
            .child(self.button("Text", "find-text", cx))
            .child(self.button("Commands", "commands", cx));
        if self.request.pick {
            toolbar = toolbar
                .child(self.button("Choose", "choose", cx))
                .child(self.button("Cancel", "cancel", cx));
        }
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x181d25))
            .text_color(rgb(0xdce3ee))
            .text_size(px(14.))
            .child(toolbar);
        if let Some(panel) = self.panel {
            let title = match panel {
                Panel::Path => "Location",
                Panel::Filter => "Filter this folder",
                Panel::Files => "Find files",
                Panel::Text => "Find saved text",
                Panel::Palette => "Commands",
                Panel::Settings => "Preferences",
                Panel::Recovery => "Recovery",
                Panel::Line => "Go to line",
                Panel::NewFile => "New file name",
                Panel::NewFolder => "New folder name",
                Panel::Rename => "New name",
            };
            root = root.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child(title)
                    .child(Input::new(&self.query))
                    .child(self.button("Apply", "submit", cx).into_any_element())
                    .child(self.button("Close", "dismiss", cx)),
            );
            if matches!(panel, Panel::Files | Panel::Text) {
                root = root.child(
                    div()
                        .px_2()
                        .child(format!(
                            "Search folder: {}{}",
                            paths::display(self.scope.as_ref().unwrap_or(&self.current)),
                            if self.scope.is_some() {
                                " (pinned)"
                            } else {
                                ""
                            }
                        ))
                        .child(self.button("Pin / unpin scope", "scope", cx)),
                );
                root = root.child(
                    div()
                        .id("results")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .children(self.hits.iter().take(1000).enumerate().map(|(i, hit)| {
                            let path = hit.path.clone();
                            let line = hit.line;
                            let expected = hit.fingerprint;
                            div()
                                .id(("hit", i))
                                .px_2()
                                .py_1()
                                .cursor_pointer()
                                .child(format!(
                                    "{}:{}  {}",
                                    paths::display(&path),
                                    line,
                                    hit.excerpt
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open(path.clone(), false, line);
                                    if let Some(fingerprint) = expected {
                                        this.hit_versions.insert(this.open_token, fingerprint);
                                    }
                                    cx.notify();
                                }))
                        })),
                );
            }
            if panel == Panel::Palette {
                let query = self.query.read(cx).value().to_lowercase();
                root = root.child(
                    div()
                        .id("palette")
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .children(
                            glimpse::commands::COMMANDS
                                .iter()
                                .filter(|c| c.label.to_lowercase().contains(&query))
                                .map(|command| self.button(command.label, command.id, cx)),
                        ),
                );
            }
            if panel == Panel::Settings {
                root = root.child(
                    div()
                        .flex()
                        .gap_2()
                        .p_2()
                        .child(self.button("Toggle wrap", "wrap", cx))
                        .child(self.button("Toggle line numbers", "numbers", cx))
                        .child(self.button("Save preferences", "save-settings", cx)),
                );
            }
            if panel == Panel::Recovery {
                root = root.child(
                    div()
                        .id("records")
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .children(self.records.iter().enumerate().map(|(i, record)| {
                            let path = record.path.clone();
                            let history = record.history;
                            div()
                                .id(("record", i))
                                .p_2()
                                .child(format!(
                                    "Restore {}: {}",
                                    if history { "saved version" } else { "draft" },
                                    paths::display(&record.original)
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let id = this.next_id;
                                    this.next_id += 1;
                                    this.io.send(if history {
                                        Job::RestoreHistory(id, path.clone())
                                    } else {
                                        Job::Import(id, path.clone())
                                    });
                                    cx.notify();
                                }))
                        })),
                );
            }
        }
        if let Some(decision) = &self.close_pending {
            let names = self
                .documents
                .iter()
                .filter(|d| decision.documents.contains(&d.id) && d.dirty(cx))
                .map(|d| paths::display(&d.disk.path))
                .collect::<Vec<_>>()
                .join(", ");
            root =
                root.child(
                    div()
                        .flex()
                        .gap_2()
                        .p_2()
                        .child(format!("Unsaved changes: {names}"))
                        .child(self.button("Save and close", "save-close", cx))
                        .child(div().id("discard").p_2().child("Discard").on_click(
                            cx.listener(move |this, _, _, cx| this.commit_close(true, cx)),
                        ))
                        .child(self.button("Cancel", "cancel-close", cx)),
                );
        }
        if !self.closings.is_empty() {
            root = root.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child("Resolving recovery before closing")
                    .child(self.button("Retry cleanup", "retry-close", cx))
                    .child(self.button("Keep open", "cancel-close", cx)),
            );
        }
        if self.destination.is_some() {
            root = root.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child("Replace the observed destination?")
                    .child(self.button("Replace", "replace-destination", cx))
                    .child(self.button("Cancel", "cancel-destination", cx)),
            );
        }

        let query = self.filter.to_lowercase();
        let visible: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| paths::name(&entry.path).to_lowercase().contains(&query))
            .cloned()
            .collect();
        let scroll = self
            .scrolls
            .entry(self.current.clone())
            .or_default()
            .clone();
        let navigator = div()
            .id("navigator")
            .track_focus(&self.nav_focus)
            .w(px(280.))
            .h_full()
            .bg(rgb(0x202630))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| window.focus(&this.nav_focus)),
            )
            .child(
                uniform_list(
                    "entries",
                    visible.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|i| {
                                let entry = &visible[i];
                                let path = entry.path.clone();
                                let selected = this.selection.as_ref() == Some(&path);
                                div()
                                    .id(("entry", i))
                                    .h(px(28.))
                                    .px_3()
                                    .py_1()
                                    .bg(if selected {
                                        rgb(0x344c68)
                                    } else {
                                        rgb(0x202630)
                                    })
                                    .cursor_pointer()
                                    .child(format!(
                                        "{} {}",
                                        if entry.directory { "▸" } else { "·" },
                                        paths::name(&path)
                                    ))
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            this.selection = Some(path.clone());
                                            this.open(path.clone(), event.click_count() < 2, 1);
                                            cx.notify();
                                        },
                                    ))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .h_full()
                .track_scroll(scroll),
            );
        let tabs = div()
            .flex()
            .gap_1()
            .p_1()
            .children(self.documents.iter().map(|doc| {
                let id = doc.id;
                div()
                    .id(("tab", id))
                    .px_3()
                    .py_2()
                    .bg(if Some(id) == self.active {
                        rgb(0x344152)
                    } else {
                        rgb(0x222933)
                    })
                    .child(format!(
                        "{}{}{}",
                        paths::name(&doc.disk.path),
                        if doc.dirty(cx) { " •" } else { "" },
                        if doc.preview { " (preview)" } else { "" }
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = Some(id);
                        this.reading = false;
                        this.focus(window, cx);
                        cx.notify();
                    }))
            }));
        let mut content = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs);
        if !self.request.pick {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .p_1()
                    .child(self.button("New", "new", cx))
                    .child(self.button("Save", "save", cx))
                    .child(self.button("Save As", "save-as", cx))
                    .child(self.button("Save Copy", "save-copy", cx))
                    .child(self.button("Read / Edit", "read", cx))
                    .child(self.button("Line", "line", cx))
                    .child(self.button("Close tab", "close", cx))
                    .child(self.button("Reload disk", "reload", cx)),
            );
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            if self.reading {
                content = content.child(
                    div()
                        .id("reading")
                        .flex_1()
                        .overflow_y_scroll()
                        .p_4()
                        .child(TextView::markdown(
                            "markdown",
                            reading_source(&doc.input.read(cx).value()),
                            window,
                            cx,
                        )),
                );
            } else {
                content = content.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family("monospace")
                        .text_size(px(self.settings.font_size))
                        .child(
                            Input::new(&doc.input)
                                .h_full()
                                .disabled(self.request.pick || self.destination_busy),
                        ),
                );
            }
        } else {
            content = content.child(
                div()
                    .p_4()
                    .child("Select a file to preview, or open a file."),
            );
        }
        root.child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(navigator)
                .child(content),
        )
        .child(div().p_2().child(self.status.clone()))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            let key = &event.keystroke;
            if this.nav_focus.is_focused(window)
                && !key.modifiers.control
                && !key.modifiers.platform
            {
                let query = this.filter.to_lowercase();
                let rows: Vec<_> = this
                    .entries
                    .iter()
                    .filter(|entry| paths::name(&entry.path).to_lowercase().contains(&query))
                    .cloned()
                    .collect();
                let index = rows
                    .iter()
                    .position(|entry| Some(&entry.path) == this.selection.as_ref());
                match key.key.as_str() {
                    "up" | "down" if !rows.is_empty() => {
                        let next = if key.key == "down" {
                            index.map_or(0, |i| (i + 1).min(rows.len() - 1))
                        } else {
                            index.unwrap_or(0).saturating_sub(1)
                        };
                        this.selection = Some(rows[next].path.clone());
                        if let Some(scroll) = this.scrolls.get(&this.current) {
                            scroll.scroll_to_item(next, ScrollStrategy::Top);
                        }
                        cx.notify();
                        return;
                    }
                    "left" => {
                        this.command("parent", window, cx);
                        return;
                    }
                    "right" | "enter" => {
                        if let Some(path) = this.selection.clone() {
                            this.open(path, false, 1);
                        }
                        cx.notify();
                        return;
                    }
                    _ => {}
                }
            }
            if key.key == "escape" {
                if this.close_pending.is_some() || !this.closings.is_empty() {
                    this.command("cancel-close", window, cx);
                    return;
                }
                this.panel = None;
                this.hits.clear();
                this.focus(window, cx);
                cx.notify();
                return;
            }
            if key.modifiers.control || key.modifiers.platform {
                let binding =
                    glimpse::commands::bindings(&this.settings.keymaps).unwrap_or_default();
                if let Some(command) = binding
                    .iter()
                    .find(|binding| {
                        let name = match binding.key.name() {
                            "ArrowLeft" => "left",
                            "ArrowRight" => "right",
                            "ArrowUp" => "up",
                            "ArrowDown" => "down",
                            "Comma" => ",",
                            other => other,
                        };
                        name.eq_ignore_ascii_case(&key.key)
                            && binding.modifiers.shift == key.modifiers.shift
                            && binding.modifiers.alt == key.modifiers.alt
                    })
                    .and_then(|binding| {
                        glimpse::commands::COMMANDS
                            .iter()
                            .find(|command| command.action == binding.action)
                    })
                {
                    if command.id != "find" {
                        this.command(command.id, window, cx);
                        cx.stop_propagation();
                    }
                    return;
                }
                let command = match (key.key.as_str(), key.modifiers.shift) {
                    ("g", _) => "line",
                    ("e", true) => "navigator",
                    _ => return,
                };
                this.command(command, window, cx);
                cx.stop_propagation();
            }
        }))
    }
}

fn disk_path(documents: &[Document], id: u64) -> PathBuf {
    documents
        .iter()
        .find(|doc| doc.id == id)
        .unwrap()
        .disk
        .path
        .clone()
}

fn reading_source(source: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut result = String::new();
    let mut cursor = 0;
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        let replacement = match event {
            Event::Start(Tag::Image { .. }) => Some("*Image omitted in this port*".to_owned()),
            Event::Html(html) | Event::InlineHtml(html) => Some(
                html.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            ),
            _ => None,
        };
        if let Some(replacement) = replacement
            && range.start >= cursor
        {
            result.push_str(&source[cursor..range.start]);
            result.push_str(&replacement);
            cursor = range.end;
        }
    }
    result.push_str(&source[cursor..]);
    result
}
