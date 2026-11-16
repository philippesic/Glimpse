mod ui;

use crate::{
    EXIT_STATUS, appearance, cli, closing, commands, directory, document, editor, io, lifecycle,
    macos, markdown, name, native_editor, navigation, observe, reservations, search, storage,
    syntax,
};
use document::DiskFile;
use eframe::egui::{self, RichText};
use io::{IoWorker, Job, Reply, SaveDestination};
use navigation::Navigator;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

struct Document {
    id: u64,
    disk: DiskFile,
    text: String,
    saved_text: String,
    saving: bool,
    generation: u64,
    preview: bool,
    conflict: Option<String>,
    external: Option<(DiskFile, String)>,
    metadata: editor::Metadata,
    highlight: Option<syntax::Highlight>,
    restored: bool,
}
impl Document {
    fn dirty(&self) -> bool {
        self.restored
            || self.text != self.saved_text
            || (self.disk.target.as_os_str().is_empty() && self.generation > 0)
    }
}
struct Confirmation {
    id: u64,
    saving: bool,
    filename: String,
}
#[derive(Clone, Copy)]
enum Pending {
    Close(u64),
    Quit,
    Finish,
}

pub(crate) struct Glimpse {
    io: IoWorker,
    directories: directory::Directories,
    completions: directory::Directories,
    completion: Option<(u64, PathBuf, String)>,
    completed_paths: Vec<(PathBuf, bool)>,
    observer: observe::Observer,
    composing: bool,
    native: native_editor::Editors,
    use_native: bool,
    navigator: Navigator,
    documents: Vec<Document>,
    active: Option<u64>,
    next_id: u64,
    opening: HashMap<PathBuf, (bool, u64)>,
    open_token: u64,
    open_latency: Option<(PathBuf, std::time::Instant)>,
    status: Option<String>,
    save_errors: HashMap<u64, String>,
    history_warning: Option<String>,
    pending: Option<Pending>,
    allow_close: bool,
    focus_editor: bool,
    settings: storage::Settings,
    settings_open: bool,
    bindings: Vec<commands::Binding>,
    palette_open: bool,
    palette_focus: bool,
    palette_query: String,
    path_focus: bool,
    path_input: String,
    filter: String,
    operation: Option<u8>,
    operation_name: String,
    markdown: markdown::Markdown,
    syntax: syntax::Syntax,
    syntax_request: Option<(u64, u64, bool)>,
    search: search::Search,
    search_open: bool,
    search_focus: bool,
    query: String,
    content_search: bool,
    scope: Option<PathBuf>,
    temporary_scope: Option<PathBuf>,
    search_id: u64,
    hits: Vec<search::Hit>,
    search_state: String,
    find: String,
    replacement: String,
    find_open: bool,
    find_case: bool,
    find_word: bool,
    find_focus: bool,
    line: usize,
    read: bool,
    recovered: Vec<storage::Record>,
    recovered_storage: storage::RecoveryUsage,
    recovered_truncated: bool,
    recovery_open: bool,
    recovery_retry: Option<Job>,
    destinations: reservations::Reservations,
    copy_without_recovery: bool,
    started: std::time::Instant,
    measured: bool,
    compact: bool,
    last_scope: PathBuf,
    history: Vec<PathBuf>,
    future: Vec<PathBuf>,
    last_location: PathBuf,
    history_move: bool,
    wait_session: bool,
    finishing: std::collections::HashSet<u64>,
    closing: closing::Closings,
    cleanup_errors: HashMap<u64, (u64, String)>,
    quit_after_request: bool,
    was_focused: bool,
    compare_open: bool,
    replace_destination: Option<SaveDestination>,
    abandon_destination: Option<SaveDestination>,
    goto: Option<(usize, usize)>,
    goto_path: Option<PathBuf>,
    pending_hit: Option<search::Hit>,
    wait_paths: std::collections::HashSet<PathBuf>,
    wait_owners: HashMap<PathBuf, u64>,
    failed_wait: std::collections::HashSet<PathBuf>,
    request_abandoned: bool,
    startup_failed: bool,
    prepared_frame: Option<u64>,
    restoring_location: Option<PathBuf>,
}
impl Glimpse {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, initial: PathBuf) -> Self {
        let setup_started = std::time::Instant::now();
        let ctx = &cc.egui_ctx;
        macos::guard_native_quit(ctx);
        crate::native_accessibility::install(ctx);
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let settings_result = storage::Settings::load();
        let mut app = Self {
            io: IoWorker::new(ctx.clone()),
            directories: directory::Directories::new(ctx.clone()),
            completions: directory::Directories::new(ctx.clone()),
            completion: None,
            completed_paths: Vec::new(),
            observer: observe::Observer::new(ctx.clone()),
            composing: false,
            native: native_editor::Editors::new(ctx.clone(), {
                let window = cc.winit_window().cloned();

                std::sync::Arc::new(move || {
                    if let Some(window) = &window {
                        window.request_redraw();
                    }
                })
            }),
            use_native: std::env::var_os("GLIMPSE_EGUI_EDITOR").is_none(),
            navigator: Navigator::new(home, initial.clone()),
            documents: Vec::new(),
            active: None,
            next_id: 0,
            opening: HashMap::new(),
            open_token: 0,
            open_latency: None,
            status: None,
            save_errors: HashMap::new(),
            history_warning: None,
            pending: None,
            allow_close: false,
            focus_editor: false,
            settings: settings_result.clone().unwrap_or_default(),
            settings_open: false,
            bindings: commands::bindings(&settings_result.clone().unwrap_or_default().keymaps)
                .unwrap_or_default(),
            palette_open: false,
            palette_focus: false,
            palette_query: String::new(),
            path_focus: false,
            path_input: initial.display().to_string(),
            filter: String::new(),
            operation: None,
            operation_name: String::new(),
            markdown: markdown::Markdown::new(ctx.clone()),
            syntax: syntax::Syntax::new(ctx.clone()),
            syntax_request: None,
            search: search::Search::new(ctx.clone()),
            search_open: false,
            search_focus: false,
            query: String::new(),
            content_search: false,
            scope: None,
            temporary_scope: None,
            search_id: 0,
            hits: Vec::new(),
            search_state: String::new(),
            find: String::new(),
            replacement: String::new(),
            find_open: false,
            find_case: false,
            find_word: false,
            find_focus: false,
            line: 1,
            read: false,
            recovered: Vec::new(),
            recovered_storage: Default::default(),
            recovered_truncated: false,
            recovery_open: false,
            recovery_retry: None,
            destinations: Default::default(),
            copy_without_recovery: false,
            started: std::time::Instant::now(),
            measured: false,
            compact: false,
            last_scope: initial.clone(),
            history: Vec::new(),
            future: Vec::new(),
            last_location: initial.clone(),
            history_move: false,
            wait_session: false,
            finishing: Default::default(),
            closing: Default::default(),
            cleanup_errors: HashMap::new(),
            quit_after_request: false,
            was_focused: false,
            compare_open: false,
            replace_destination: None,
            abandon_destination: None,
            goto: None,
            goto_path: None,
            pending_hit: None,
            wait_paths: Default::default(),
            wait_owners: HashMap::new(),
            failed_wait: Default::default(),
            request_abandoned: false,
            startup_failed: false,
            prepared_frame: None,
            restoring_location: None,
        };
        if let Some(warning) = app.settings.warning.clone() {
            app.status = Some(warning);
        }
        if let Err(error) = settings_result {
            app.status = Some(error);
        }
        app.compact = app.settings.compact;
        appearance::install(
            ctx,
            if app.compact {
                appearance::Density::Compact
            } else {
                appearance::Density::Comfortable
            },
        );
        ctx.set_zoom_factor(app.settings.ui_scale);
        ctx.set_theme(if app.settings.system_theme {
            egui::ThemePreference::System
        } else if app.settings.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        app.io.send(Job::Inventory);
        app.flush_navigation();
        if std::env::var_os("GLIMPSE_MEASURE").is_some() {
            eprintln!(
                "APP_SETUP_MS {:.3}",
                setup_started.elapsed().as_secs_f64() * 1000.
            );
        }
        app
    }
    pub(crate) fn configure_request(&mut self, request: cli::Request, started: std::time::Instant) {
        self.started = started;
        if request.desktop && self.settings.remember_locations && !self.settings.recents.is_empty()
        {
            self.restoring_location = Some(self.navigator.current.clone());
            self.io
                .send(Job::RecentLocation(self.settings.recents.clone()));
        }
        self.scope = request.scope;
        self.wait_session = request.wait;
        if request.wait {
            self.wait_paths = request.files.iter().cloned().collect();
        }
        if !request.files.is_empty() {
            self.navigator.leave_for_editor();
        }
        self.goto_path = request.files.first().cloned();
        for file in request.files.into_iter().rev() {
            self.open(file, true);
        }
        self.line = request.line;
        self.goto = if request.line > 1 || request.column > 1 {
            Some((request.line, request.column))
        } else {
            None
        };
    }
    fn flush_navigation(&mut self) {
        self.navigator.cancel_inactive();
        self.directories.retain(&self.navigator.requested_folders());
        for path in self.navigator.take_requests() {
            match self.directories.request(path.clone()) {
                Ok(generation) => self.navigator.begin(path, generation),
                Err(error) => self.navigator.receive(path, Err(error)),
            }
        }
    }
    fn choose_folder(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new()
            .set_title("Go to Folder")
            .set_directory(&self.navigator.current);
        if let Some(path) = dialog.pick_folder() {
            self.navigator.navigate(path);
            ctx.memory_mut(|m| m.request_focus(egui::Id::unique("navigator")));
        }
    }
    fn remember_document_location(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if self.settings.record_location(parent, now) {
                self.io.send(Job::Settings(self.settings.clone()));
            }
        }
    }

    fn open(&mut self, path: PathBuf, focus_editor: bool) {
        self.open_latency = Some((path.clone(), std::time::Instant::now()));
        self.read = false;
        self.open_token += 1;
        let token = self.open_token;
        self.io.cancel_previews(token);
        if let Some(doc) = self.documents.iter().find(|d| d.disk.path == path) {
            self.active = Some(doc.id);
            self.focus_editor = true;
            self.navigator.leave_for_editor();
            self.navigator.reveal_document(&path);
            self.remember_document_location(&path);
            return;
        }
        self.opening.insert(path.clone(), (focus_editor, token));
        self.io.send(Job::Open(path, token, !focus_editor));
    }
    fn command(&mut self, action: commands::Action, ctx: &egui::Context) {
        use commands::Action::*;
        let selected = || {
            self.navigator
                .selection()
                .unwrap_or(self.navigator.current.clone())
        };
        match action {
            Choose => {
                self.status = Some("Choose Selection is available in a picker request.".into())
            }
            Cancel => {
                self.pending = None;
                self.palette_open = false;
                self.settings_open = false;
                self.recovery_open = false;
                self.recovery_retry = None;
                self.search_open = false;
                self.temporary_scope = None;
                self.find_open = false;
                self.operation = None;
                self.completed_paths.clear();
                self.focus_editor = true;
                for destination in [
                    self.replace_destination.take(),
                    self.abandon_destination.take(),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == destination.id) {
                        doc.saving = false;
                    }
                }
            }
            RetryCleanup => self.retry_cleanup(),
            CancelClosing => self.cancel_closing(),
            RetainCleanup => self.retain_cleanup_records(),
            New => self.new_document(),
            Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.navigator.current)
                    .pick_file()
                {
                    self.open(path, true);
                }
            }
            Folder => self.choose_folder(ctx),
            Save => {
                if let Some(id) = self.active {
                    self.save(id);
                }
            }
            SaveAs => self.save_to(false),
            SaveCopy => self.save_to(true),
            Close => {
                if let Some(id) = self.active {
                    self.request_close(id);
                }
            }
            Quit => self.pending = Some(Pending::Quit),
            Back => {
                if let Some(path) = self.history.pop() {
                    self.future.push(self.navigator.current.clone());
                    self.history_move = true;
                    self.navigator.navigate(path);
                }
            }
            Forward => {
                if let Some(path) = self.future.pop() {
                    self.history.push(self.navigator.current.clone());
                    self.history_move = true;
                    self.navigator.navigate(path);
                }
            }
            Home => self.navigator.home(),
            Parent => self.navigator.up(),
            Refresh => self.navigator.refresh(),
            Files | Text => {
                self.find_open = false;
                self.content_search = action == Text;
                self.search_open = true;
                self.search_focus = true;
                self.focus_editor = false;
                self.navigator.focused = false;
                self.native.focus_host();
            }
            Find => {
                self.search_open = false;
                self.search_id = self.search.cancel();
                self.temporary_scope = None;
                self.navigator.focused = false;
                self.find_open = true;
                self.find_focus = true;
                self.focus_editor = false;
                self.native.focus_host();
            }
            Read => {
                self.read = !self.read;
                self.focus_editor = !self.read;
                if self.read {
                    self.native.focus_host();
                    self.markdown.refresh();
                }
            }
            Settings => self.settings_open = true,
            Recovery => {
                self.recovery_open = true;
                self.io.send(Job::Inventory);
            }
            Palette => {
                self.palette_open = true;
                self.palette_focus = true;
                self.palette_query.clear();
            }
            Path => {
                self.path_focus = true;
                self.focus_editor = false;
                self.native.focus_host();
            }
            Pin => {
                if self.settings.pins.contains(&self.navigator.current) {
                    self.settings.pins.retain(|p| p != &self.navigator.current);
                } else {
                    self.settings.pins.push(self.navigator.current.clone());
                }
                self.io.send(Job::Settings(self.settings.clone()));
            }
            Scope => {
                self.temporary_scope = None;
                self.scope = if self.scope.is_some() {
                    None
                } else {
                    Some(self.navigator.current.clone())
                };
                self.search_id = self.search.cancel();
                self.hits.clear();
            }
            GoScope => {
                if let Some(path) = self.scope.clone() {
                    self.navigator.navigate(path);
                }
            }
            CopyPath => {
                self.status = macos::copy_path(&selected()).err();
            }
            CopyRelative => {
                match selected().strip_prefix(
                    self.temporary_scope
                        .as_ref()
                        .or(self.scope.as_ref())
                        .unwrap_or(&self.navigator.current),
                ) {
                    Ok(path) => ctx.copy_text(path.display().to_string()),
                    Err(_) => self.status = Some("Selection is outside the search folder.".into()),
                }
            }
            Reveal => {
                if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active)
                    && let Some(parent) = doc.disk.path.parent()
                {
                    self.navigator.navigate(parent.to_path_buf());
                    self.navigator.leave_for_editor();
                    self.navigator.reveal_document(&doc.disk.path);
                    self.navigator.focused = true;
                    self.focus_editor = false;
                    self.native.focus_host();
                }
            }
            External => self.io.send(Job::External(selected(), false)),
            Finder => self.io.send(Job::External(selected(), true)),
            NewFolder | NamedFile | Rename | Trash => {
                self.operation = Some(match action {
                    NewFolder => 0,
                    NamedFile => 1,
                    Rename => 2,
                    _ => 3,
                });
                self.operation_name = if action == Rename {
                    self.navigator
                        .selection()
                        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
            }
            Wrap => {
                self.settings.wrap = !self.settings.wrap;
                self.io.send(Job::Settings(self.settings.clone()));
            }
            Indent | Outdent => {
                if let Some(id) = self.active {
                    self.native.indent(id, action == Outdent);
                }
            }
            Finish => self.finish_request(),
            Abort => {
                EXIT_STATUS.store(1, std::sync::atomic::Ordering::Relaxed);
                self.pending = Some(Pending::Quit);
            }
        }
    }
    fn new_document(&mut self) {
        self.read = false;
        let id = self.next_id;
        self.next_id += 1;
        self.documents.push(Document {
            id,
            disk: DiskFile::draft(),
            text: String::new(),
            saved_text: String::new(),
            saving: false,
            generation: 0,
            preview: false,
            conflict: None,
            external: None,
            metadata: editor::Metadata::default(),
            highlight: None,
            restored: false,
        });
        self.active = Some(id);
        self.focus_editor = true;
    }
    fn save_to(&mut self, copy: bool) {
        self.reconcile_destinations();
        if self
            .active
            .is_some_and(|id| self.closing.contains(id) || self.destinations.contains(id))
        {
            self.status =
                Some("Wait for closing recovery cleanup, or cancel closing first.".into());
            return;
        }
        let requested = self.active.is_some_and(|id| self.requested(id));
        let without_recovery = std::mem::take(&mut self.copy_without_recovery);
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| Some(d.id) == self.active)
            && !doc.saving
            && let Some(path) = rfd::FileDialog::new()
                .set_title(if copy { "Save Copy" } else { "Save As" })
                .set_directory(&self.navigator.current)
                .save_file()
        {
            doc.saving = true;
            let destination = SaveDestination {
                id: doc.id,
                path,
                text: doc.text.clone(),
                copy,
                bom: doc.disk.bom,
                generation: doc.generation,
                expected: None,
                original: doc.disk.clone(),
                without_recovery,
            };
            if !copy && requested && destination.path != doc.disk.path {
                self.abandon_destination = Some(destination);
            } else {
                self.io.send(Job::Prepare(destination));
            }
        }
    }
    fn save(&mut self, id: u64) {
        if self.closing.contains(id) || self.destinations.contains(id) {
            self.status =
                Some("Wait for closing recovery cleanup, or cancel closing first.".into());
            return;
        }
        if self.documents.iter().any(|d| {
            d.id == id
                && self
                    .destinations
                    .conflicts(id, &d.disk.path, Some(&d.disk.target))
        }) {
            self.status =
                Some("Another save is using this destination. Wait for it to finish.".into());
            return;
        }
        if self
            .documents
            .iter()
            .any(|d| d.id == id && d.disk.target.as_os_str().is_empty())
        {
            self.active = Some(id);
            self.save_to(false);
            return;
        }

        if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id)
            && !doc.saving
        {
            doc.saving = true;
            self.status = None;
            self.io.send(Job::Save {
                id,
                disk: doc.disk.clone(),
                text: doc.text.clone(),
                generation: doc.generation,
                without_recovery: false,
            });
        }
    }
    fn reserve_destination(
        &mut self,
        destination: &SaveDestination,
        expected: Option<&DiskFile>,
    ) -> Result<(), String> {
        let mut aliases = Vec::new();
        for doc in &self.documents {
            if doc.id != destination.id
                && (doc.disk.path == destination.path
                    || expected.is_some_and(|disk| doc.disk.same_file(disk)))
            {
                if doc.dirty() || doc.saving || self.native.is_locked(doc.id) {
                    return Err("Destination is open with unsaved edits or another operation pending; its buffer was preserved.".into());
                }
                aliases.push((doc.id, doc.generation));
            }
        }
        self.destinations
            .reserve(
                destination.id,
                reservations::Reservation {
                    path: destination.path.clone(),
                    target: expected.map(|d| d.target.clone()),
                    aliases: aliases.clone(),
                },
            )
            .map_err(str::to_owned)?;
        for (id, _) in aliases {
            self.native.set_locked(id, true);
        }
        Ok(())
    }
    fn release_destination(&mut self, owner: u64) {
        if let Some(reservation) = self.destinations.take(owner) {
            for (id, _) in reservation.aliases {
                self.native.set_locked(
                    id,
                    self.closing.contains(id) || self.destinations.contains(id),
                );
            }
        }
    }
    fn reconcile_destinations(&mut self) {
        for owner in self.destinations.owners() {
            let retry = matches!(&self.recovery_retry,Some(Job::SaveTo(d)) if d.id==owner);
            if !retry && !self.documents.iter().any(|d| d.id == owner && d.saving) {
                self.release_destination(owner);
            }
        }
    }
    fn complete_destination_aliases(
        &mut self,
        destination: &SaveDestination,
        disk: &DiskFile,
        ctx: &egui::Context,
    ) {
        let Some(reservation) = self.destinations.take(destination.id) else {
            return;
        };
        for (id, generation) in reservation.aliases {
            let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) else {
                continue;
            };
            if doc.generation != generation || doc.dirty() {
                doc.conflict=Some("Destination was saved while this buffer changed. Local edits retained; check the disk version.".into());
                self.native.set_locked(id, self.closing.contains(id));
                continue;
            }
            if destination.copy {
                editor::replace(
                    ctx,
                    egui::Id::unique(("document", id)),
                    &doc.text,
                    &destination.text,
                );
                let alias = doc.disk.path.clone();
                doc.disk = disk.clone();
                doc.disk.path = alias;
                doc.text = destination.text.clone();
                doc.saved_text = destination.text.clone();
                doc.generation += 1;
                doc.external = None;
                doc.conflict = None;
                self.native.set_locked(id, false);
            } else {
                for owner in self.wait_owners.values_mut() {
                    if *owner == id {
                        *owner = destination.id;
                    }
                }
                if self.active == Some(id) {
                    self.active = Some(destination.id);
                    self.focus_editor = true;
                }
                self.begin_close(id, false);
            }
        }
    }
    fn receive(&mut self, ctx: &egui::Context) {
        for index in 0..2 {
            let Ok(batch) = self.directories.replies.try_recv() else {
                break;
            };
            self.navigator.batch(batch);
            if index == 1 {
                ctx.request_repaint();
            }
        }

        for index in 0..8 {
            let Ok(reply) = self.io.replies.try_recv() else {
                break;
            };
            if index == 7 {
                ctx.request_repaint();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint();
            match reply {
                Reply::RecentLocation(path) => {
                    if self
                        .restoring_location
                        .take()
                        .is_some_and(|initial| initial == self.navigator.current)
                        && let Some(path) = path
                    {
                        self.navigator.navigate(path);
                    }
                }
                Reply::Cleared(id, generation, result) => {
                    self.cleanup_acknowledged(id, generation, result)
                }
                Reply::Chosen(_, _) => {}
                Reply::OpenDirectory(path, token) => {
                    if self
                        .opening
                        .get(&path)
                        .is_some_and(|(_, expected)| *expected == token)
                    {
                        self.opening.remove(&path);
                        if token == self.open_token {
                            self.navigator.navigate(path);
                        }
                    }
                }
                Reply::Finished(id, generation, result) => {
                    self.finishing.remove(&id);
                    match result {
                        Ok(()) => {
                            if self
                                .documents
                                .iter()
                                .any(|d| d.id == id && d.generation == generation && !d.dirty())
                            {
                                self.remove(id);
                            } else {
                                self.status = Some(
                                    "Document changed while finishing; request remains waiting."
                                        .into(),
                                );
                            }
                        }
                        Err(error) => {
                            self.status = Some(format!(
                                "Cannot finish requested destination: {error}. Retry Finish, Save Copy, or Abort."
                            ))
                        }
                    }
                }
                Reply::RecordTrashed(result) => {
                    if let Err(e) = result {
                        self.status = Some(format!("Record retained: {e}"));
                    }
                    self.io.send(Job::Inventory);
                }
                Reply::SettingsLoaded(result) => match result {
                    Ok(settings) => {
                        self.bindings = commands::bindings(&settings.keymaps).unwrap_or_default();
                        self.compact = settings.compact;
                        self.status = settings.warning.clone();
                        self.settings = settings;
                        ctx.set_zoom_factor(self.settings.ui_scale);
                        ctx.set_theme(if self.settings.system_theme {
                            egui::ThemePreference::System
                        } else if self.settings.dark {
                            egui::ThemePreference::Dark
                        } else {
                            egui::ThemePreference::Light
                        });
                    }
                    Err(e) => self.status = Some(e),
                },
                Reply::Records(result) => match result {
                    Ok(inventory) => {
                        self.recovered = inventory.records;
                        self.recovered_storage = inventory.usage;
                        self.recovered_truncated = inventory.records_truncated;
                    }
                    Err(e) => self.history_warning = Some(format!("Cannot inspect recovery: {e}")),
                },
                Reply::HistoryWarning(error) => {
                    self.history_warning =
                        Some(format!("Recovery storage needs attention: {error}"))
                }
                Reply::Prepared(mut destination, result) => match result {
                    Ok(Some(expected)) => {
                        if destination.copy && expected.same_file(&destination.original) {
                            if let Some(doc) =
                                self.documents.iter_mut().find(|d| d.id == destination.id)
                            {
                                doc.saving = false;
                            }
                            self.status=Some("Save Copy requires a separate destination. Use Save for this file.".into());
                            continue;
                        }
                        if let Err(error) = self.reserve_destination(&destination, Some(&expected))
                        {
                            self.status = Some(error);
                            if let Some(doc) =
                                self.documents.iter_mut().find(|d| d.id == destination.id)
                            {
                                doc.saving = false;
                            }
                        } else {
                            destination.expected = Some(expected);
                            self.replace_destination = Some(*destination);
                        }
                    }
                    Ok(None) => {
                        if let Err(error) = self.reserve_destination(&destination, None) {
                            self.status = Some(error);
                            if let Some(doc) =
                                self.documents.iter_mut().find(|d| d.id == destination.id)
                            {
                                doc.saving = false;
                            }
                        } else {
                            self.io.send(Job::SaveTo(*destination));
                        }
                    }
                    Err(error) => {
                        if let Some(doc) =
                            self.documents.iter_mut().find(|d| d.id == destination.id)
                        {
                            doc.saving = false;
                        }
                        self.status = Some(error);
                    }
                },
                Reply::Imported(id, result) => {
                    self.io.send(Job::Inventory);
                    match result {
                        Ok((draft, mut disk)) => {
                            if self.documents.iter().any(|d| {
                                d.disk.same_file(&disk)
                                    && (d.dirty() || self.native.is_locked(d.id))
                            }) {
                                let mut unbound = DiskFile::draft();
                                unbound.path = disk.path.clone();
                                unbound.bytes = disk.bytes.clone();
                                unbound.bom = disk.bom;
                                disk = unbound;
                                self.status=Some("An alias became dirty while restoring. Recovered text is an unbound draft; Save As chooses its destination.".into());
                            } else {
                                let old: Vec<_> = self
                                    .documents
                                    .iter()
                                    .filter(|d| d.disk.same_file(&disk))
                                    .map(|d| d.id)
                                    .collect();
                                for id in old {
                                    self.remove(id);
                                }
                            }
                            let saved_text = String::from_utf8_lossy(if disk.bom {
                                disk.bytes.get(3..).unwrap_or_default()
                            } else {
                                &disk.bytes
                            })
                            .into_owned();
                            self.documents.push(Document {
                                id,
                                disk,
                                text: draft.text,
                                saved_text,
                                saving: false,
                                generation: draft.generation.max(1),
                                preview: false,
                                conflict: Some(
                                    "Recovered local draft. Check Disk Version before saving."
                                        .into(),
                                ),
                                external: None,
                                metadata: editor::Metadata::default(),
                                highlight: None,
                                restored: true,
                            });
                            self.active = Some(id);
                            self.focus_editor = true;
                        }
                        Err(error) => self.status = Some(error),
                    }
                }
                Reply::Visit(path, folder) => {
                    if folder {
                        self.navigator.navigate(path);
                    } else {
                        if let Some(parent) = path.parent() {
                            self.navigator.navigate(parent.to_path_buf());
                        }
                        self.open(path, true);
                    }
                }
                Reply::Done(result) => {
                    if let Err(error) = result {
                        self.status = Some(error);
                    }
                }
                Reply::Changed(result) => match result {
                    Ok(()) => self.navigator.refresh(),
                    Err(e) => {
                        self.status = Some(e);
                    }
                },
                Reply::SavedTo(destination, result) => {
                    let id = destination.id;
                    let text = destination.text.clone();
                    let copy = destination.copy;
                    if let Ok(disk) = &result {
                        self.navigator.refresh();
                        if !copy
                            && self
                                .documents
                                .iter()
                                .any(|d| d.id == id && !d.disk.same_file(disk))
                        {
                            let abandoned: Vec<_> = self
                                .wait_owners
                                .iter()
                                .filter(|(_, owner)| **owner == id)
                                .map(|(path, _)| path.clone())
                                .collect();
                            if !abandoned.is_empty() {
                                self.request_abandoned = true;
                                EXIT_STATUS.store(
                                    if self.failed_wait.is_empty() { 1 } else { 2 },
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                                for path in abandoned {
                                    self.wait_paths.remove(&path);
                                    self.wait_owners.remove(&path);
                                }
                            }
                        }
                        self.complete_destination_aliases(&destination, disk, ctx);
                    }
                    if result
                        .as_ref()
                        .is_err_and(|error| !error.starts_with("Recovery unavailable:"))
                    {
                        self.release_destination(id);
                    }
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                        doc.saving = false;
                        match result {
                            Ok(disk) => {
                                self.save_errors.remove(&id);
                                if !copy {
                                    doc.disk = disk;
                                    doc.generation += 1;
                                    doc.external = None;
                                    doc.saved_text = text;
                                    doc.restored = false;
                                    doc.conflict = None;
                                }
                                self.status = Some(
                                    if copy {
                                        "Copy saved"
                                    } else {
                                        "Saved to destination"
                                    }
                                    .into(),
                                );
                            }
                            Err(e) => {
                                if e.starts_with("Recovery unavailable:") {
                                    self.recovery_retry = Some(Job::SaveTo(*destination));
                                }
                                let message = format!("Cannot save destination: {e}");
                                self.save_errors.insert(id, message.clone());
                                self.status = Some(message);
                            }
                        }
                    }
                }
                Reply::Checked(id, generation, result) => {
                    if let Some(doc) = self.documents.iter_mut().find(|d| {
                        d.id == id
                            && d.generation == generation
                            && !d.saving
                            && !self.native.is_locked(id)
                    }) {
                        match result {
                            Ok((disk, text)) => {
                                if disk.target == doc.disk.target && disk.bytes == doc.disk.bytes {
                                    doc.disk = disk;
                                    doc.conflict = None;
                                    doc.external = None;
                                } else if disk.target == doc.disk.target
                                    && !doc.dirty()
                                    && !self.composing
                                    && !ctx.input(|i| i.pointer.any_down())
                                {
                                    editor::replace(
                                        ctx,
                                        egui::Id::unique(("document", doc.id)),
                                        &doc.text,
                                        &text,
                                    );
                                    doc.text = text.clone();
                                    doc.saved_text = text;
                                    doc.disk = disk;
                                    doc.generation += 1;
                                    doc.conflict = None;
                                    self.status = Some(
                                        "Disk version refreshed; Undo restores the previous text."
                                            .into(),
                                    );
                                } else {
                                    doc.external = Some((disk.clone(), text.clone()));
                                    doc.conflict = Some(format!(
                                        "Disk differs ({} bytes). Local buffer retained. Compare versions or save a copy.",
                                        text.len()
                                    ));
                                }
                            }
                            Err(e) => doc.conflict = Some(format!("Disk unavailable: {e}")),
                        }
                    }
                }

                Reply::Open(path, token, result) => {
                    if self
                        .opening
                        .get(&path)
                        .is_none_or(|(_, expected)| *expected != token)
                    {
                        continue;
                    }
                    let (focus_editor, _) = self.opening.remove(&path).unwrap();
                    let activate = token == self.open_token;
                    if !focus_editor && !activate {
                        continue;
                    }
                    match result {
                        Ok((disk, text)) => {
                            if self.destinations.blocks(&path, Some(&disk.target)) {
                                self.status=Some("This destination is being saved. Open it again after the save finishes.".into());
                                continue;
                            }
                            if self.pending_hit.as_ref().is_some_and(|h| h.path == path) {
                                let hit = self.pending_hit.take().unwrap();
                                let dirty = self
                                    .documents
                                    .iter()
                                    .any(|d| d.disk.same_file(&disk) && d.dirty());
                                let valid = !dirty
                                    && hit
                                        .fingerprint
                                        .is_none_or(|v| v == search::fingerprint(&disk.bytes))
                                    && (hit.fingerprint.is_none()
                                        || editor::line_ranges(&text)
                                            .nth(hit.line.saturating_sub(1))
                                            .is_some_and(|range| text[range].contains(&hit.query)));
                                if valid {
                                    self.goto = Some((hit.line, 1));
                                    self.goto_path = Some(path.clone());
                                } else {
                                    self.status=Some("Search hit is stale or the buffer has unsaved edits. Buffer preserved; search again for a current line.".into());
                                }
                            }
                            if let Some(doc) =
                                self.documents.iter().find(|d| d.disk.same_file(&disk))
                            {
                                if self.wait_paths.contains(&path) || self.failed_wait.remove(&path)
                                {
                                    self.wait_paths.insert(path.clone());
                                    self.wait_owners.insert(path.clone(), doc.id);
                                    if self.failed_wait.is_empty() {
                                        EXIT_STATUS.store(
                                            if self.request_abandoned { 1 } else { 0 },
                                            std::sync::atomic::Ordering::Relaxed,
                                        );
                                    }
                                }
                                if self.goto_path.as_ref() == Some(&path) {
                                    self.goto_path = Some(doc.disk.path.clone());
                                }
                                if activate {
                                    self.active = Some(doc.id);
                                }
                            } else {
                                if !focus_editor {
                                    self.documents
                                        .retain(|d| !d.preview || d.dirty() || d.saving);
                                }
                                let id = self.next_id;
                                self.next_id += 1;
                                self.documents.push(Document {
                                    id,
                                    disk,
                                    text: text.clone(),
                                    saved_text: text,
                                    saving: false,
                                    generation: 0,
                                    preview: !focus_editor,
                                    conflict: None,
                                    external: None,
                                    metadata: editor::Metadata::default(),
                                    highlight: None,
                                    restored: false,
                                });
                                if self.wait_paths.contains(&path) || self.failed_wait.remove(&path)
                                {
                                    self.wait_paths.insert(path.clone());
                                    self.wait_owners.insert(path.clone(), id);
                                    if self.failed_wait.is_empty() {
                                        EXIT_STATUS.store(
                                            if self.request_abandoned { 1 } else { 0 },
                                            std::sync::atomic::Ordering::Relaxed,
                                        );
                                    }
                                }
                                if activate {
                                    self.active = Some(id);
                                }
                            }
                            if activate {
                                self.focus_editor = true;
                                self.navigator.leave_for_editor();
                                self.navigator.reveal_document(&path);
                                self.remember_document_location(&path);
                            }
                        }
                        Err(error) => {
                            if error == "Cancelled" {
                                continue;
                            }
                            if !self.measured {
                                self.startup_failed = true;
                            }
                            if self.wait_paths.remove(&path) {
                                self.failed_wait.insert(path.clone());
                                EXIT_STATUS.store(2, std::sync::atomic::Ordering::Relaxed);
                            }
                            self.status = Some(format!("Cannot open {}: {error}", name(&path)));
                        }
                    }
                }
                Reply::Save(id, text, result) => {
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                        doc.saving = false;
                        match result {
                            Ok(disk) => {
                                doc.disk = disk;
                                doc.generation += 1;
                                doc.conflict = None;
                                doc.external = None;
                                doc.saved_text = text;
                                doc.restored = false;
                                self.save_errors.remove(&id);
                                self.status = None;
                            }
                            Err(error) => {
                                if error.starts_with("Recovery unavailable:") {
                                    self.recovery_retry = Some(Job::Save {
                                        id: doc.id,
                                        disk: doc.disk.clone(),
                                        text: doc.text.clone(),
                                        generation: doc.generation,
                                        without_recovery: false,
                                    });
                                }
                                let message =
                                    format!("Cannot save {}: {error}", name(&doc.disk.path));
                                self.save_errors.insert(id, message.clone());
                                self.status = Some(message);
                            }
                        }
                    }
                }
            }
        }
    }
    fn remove(&mut self, id: u64) {
        self.begin_close(id, false);
    }
    fn begin_close(&mut self, id: u64, discard: bool) {
        if self.destinations.contains(id) {
            self.status = Some(
                "This tab is reserved by a destination save. Wait for that operation to finish."
                    .into(),
            );
            return;
        }
        if let Some(doc) = self.documents.iter().find(|d| d.id == id && !d.saving) {
            let generation = doc.generation;
            if self.closing.begin(id, generation, discard) {
                self.native.set_locked(id, true);
                self.io.send(Job::ClearThrough(id, generation));
                self.status =
                    Some("Clearing the document's recovery record before closing…".into());
            }
        }
    }
    fn cleanup_acknowledged(&mut self, id: u64, generation: u64, result: Result<(), String>) {
        if !self.closing.contains(id) {
            if let Err(error) = result {
                self.cleanup_errors.insert(id, (generation, error.clone()));
                self.history_warning = Some(format!(
                    "Recovery record cleanup failed: {error}. Use Retry Recovery Cleanup."
                ));
            } else if self
                .cleanup_errors
                .get(&id)
                .is_some_and(|(g, _)| *g == generation)
            {
                self.cleanup_errors.remove(&id);
            }
            return;
        }
        let current = self
            .documents
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.generation)
            .unwrap_or(generation);
        match self.closing.acknowledge(id, generation, current, result) {
            closing::Outcome::Completed { discard } => {
                if discard {
                    self.abandon_request(id);
                }
                self.native.set_locked(id, false);
                self.forget_document(id);
                self.status = None;
            }
            closing::Outcome::Changed => {
                self.native.set_locked(id, false);
                self.pending = None;
                self.quit_after_request = false;
                self.focus_editor = true;
                self.status=Some("Document changed during cleanup. Close was cancelled; the current buffer is retained.".into());
            }
            closing::Outcome::Failed(error) => {
                self.status = Some(format!(
                    "Cannot clear recovery record: {error}. Use Retry Recovery Cleanup, Cancel Closing Documents, or Close Retaining Recovery Records."
                ))
            }
            closing::Outcome::Ignored => {}
        }
    }
    fn retry_cleanup(&mut self) {
        for (id, generation) in self.closing.retry() {
            self.io.send(Job::ClearThrough(id, generation));
        }
        for (id, (generation, _)) in &self.cleanup_errors {
            self.io.send(Job::ClearThrough(*id, *generation));
        }
    }
    fn cancel_closing(&mut self) {
        for id in self.closing.cancel() {
            self.native.set_locked(id, false);
            if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                doc.generation += 1;
                self.native.acknowledge(id, doc.generation);
                if doc.dirty() {
                    self.io.send(Job::Recover(
                        id,
                        storage::Draft {
                            path: doc.disk.path.clone(),
                            text: doc.text.clone(),
                            base: doc.disk.bytes.clone(),
                            target: Some(doc.disk.target.clone()),
                            generation: doc.generation,
                        },
                    ));
                }
            }
        }
        self.pending = None;
        self.quit_after_request = false;
        self.focus_editor = true;
        self.status = Some("Closing cancelled. Buffers and undo history remain open.".into());
    }
    fn retain_cleanup_records(&mut self) {
        for (id, discard) in self.closing.failed() {
            if discard {
                self.abandon_request(id);
            }
            self.closing.forget(id);
            self.native.set_locked(id, false);
            self.forget_document(id);
        }
        self.status=Some("Documents closed with failed recovery records retained. They may be offered after restart.".into());
    }

    fn forget_document(&mut self, id: u64) {
        if let Some(index) = self.documents.iter().position(|d| d.id == id) {
            let paths: Vec<_> = self
                .wait_owners
                .iter()
                .filter(|(_, owner)| **owner == id)
                .map(|(path, _)| path.clone())
                .collect();
            for path in paths {
                self.wait_paths.remove(&path);
                self.wait_owners.remove(&path);
            }
            self.documents.remove(index);
            self.save_errors.remove(&id);
            if self.active == Some(id) {
                self.active = self
                    .documents
                    .get(index.min(self.documents.len().saturating_sub(1)))
                    .map(|d| d.id);
                self.focus_editor = true;
            }
        }
    }
    fn requested(&self, id: u64) -> bool {
        self.wait_owners.values().any(|owner| *owner == id)
    }
    fn abandon_request(&mut self, id: u64) {
        if self.requested(id) {
            self.request_abandoned = true;
            EXIT_STATUS.store(
                if self.failed_wait.is_empty() { 1 } else { 2 },
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }
    fn finish_request(&mut self) {
        if self.wait_session {
            self.pending = Some(Pending::Finish);
        } else {
            self.status = Some("This window has no waiting CLI request.".into());
        }
    }
    fn validate_finish(&mut self, id: u64) {
        if let Some(doc) = self
            .documents
            .iter()
            .find(|d| d.id == id && !self.closing.contains(id))
            && self.finishing.insert(id)
        {
            let paths = self
                .wait_owners
                .iter()
                .filter(|(_, owner)| **owner == id)
                .map(|(path, _)| path.clone())
                .collect();
            self.io
                .send(Job::Finish(id, doc.generation, doc.disk.clone(), paths));
        }
    }
    fn request_close(&mut self, id: u64) {
        if self
            .documents
            .iter()
            .any(|d| d.id == id && (d.dirty() || d.saving))
        {
            self.pending = Some(Pending::Close(id));
        } else if self.requested(id) {
            self.validate_finish(id);
        } else {
            self.remove(id);
        }
    }
    fn pending_document(&mut self, ctx: &egui::Context) -> Option<Confirmation> {
        let pending = self.pending?;
        let doc = match pending {
            Pending::Close(id) => self
                .documents
                .iter()
                .find(|d| d.id == id && !self.closing.contains(d.id) && (d.dirty() || d.saving)),
            Pending::Quit => self
                .documents
                .iter()
                .find(|d| !self.closing.contains(d.id) && (d.dirty() || d.saving)),
            Pending::Finish => self.documents.iter().find(|d| {
                self.requested(d.id) && !self.closing.contains(d.id) && (d.dirty() || d.saving)
            }),
        };
        if let Some(doc) = doc {
            return Some(Confirmation {
                id: doc.id,
                saving: doc.saving,
                filename: name(&doc.disk.path),
            });
        }
        self.pending = None;
        match pending {
            Pending::Close(id) => {
                if self.requested(id) {
                    self.validate_finish(id);
                } else {
                    self.remove(id);
                }
            }
            Pending::Finish => {
                let ids: std::collections::HashSet<_> =
                    self.wait_owners.values().copied().collect();
                for id in ids {
                    self.validate_finish(id);
                }
            }
            Pending::Quit if !self.wait_paths.is_empty() => {
                self.quit_after_request = true;
                self.pending = Some(Pending::Finish);
                ctx.request_repaint();
            }
            Pending::Quit => {
                if !self.closing.is_empty() || self.io.pending_mutations() > 0 {
                    self.pending = Some(Pending::Quit);
                    return None;
                }
                let ids: Vec<_> = self.documents.iter().map(|d| d.id).collect();
                if !ids.is_empty() {
                    for id in ids {
                        self.begin_close(id, false);
                    }
                    self.pending = Some(Pending::Quit);
                    return None;
                }
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        None
    }
    fn confirm_choice(&mut self, id: u64, choice: u8) {
        match choice {
            0 => self.save(id),
            1 => {
                self.begin_close(id, true);
            }
            2 => {
                self.pending = None;
                self.quit_after_request = false;
                self.focus_editor = true;
            }
            _ => {}
        }
    }

    fn after_frame(&mut self, ctx: &egui::Context) {
        if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() {
            eprintln!(
                "UI_FRAME {} {:.3}",
                ctx.cumulative_frame_nr(),
                self.started.elapsed().as_secs_f64()
            );
        }
        if let Some((path, started)) = &self.open_latency
            && self.documents.iter().any(|doc| {
                Some(doc.id) == self.active
                    && doc.disk.path == *path
                    && (!self.use_native || self.native.drawn(doc.id))
            })
        {
            if std::env::var_os("GLIMPSE_MEASURE").is_some() {
                eprintln!(
                    "FILE_UI_READY_MS {:.3} {}",
                    started.elapsed().as_secs_f64() * 1000.,
                    crate::paths::display(path)
                );
            }
            self.open_latency = None;
        }
        self.reconcile_destinations();
        if self.wait_session
            && self.measured
            && self.wait_paths.is_empty()
            && self.opening.is_empty()
            && self.pending.is_none()
        {
            let status = EXIT_STATUS.load(std::sync::atomic::Ordering::Relaxed);
            if let Err(error) = lifecycle::notify(b'0' + status as u8) {
                self.status = Some(format!("Cannot report request completion: {error}"));
            }
            self.wait_session = false;
            if self.quit_after_request {
                self.pending = Some(Pending::Quit);
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
                ctx.request_repaint();
            } else if self.documents.is_empty() && self.io.pending_mutations() == 0 {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if !self.measured
            && self.restoring_location.is_none()
            && self.opening.is_empty()
            && (!self.documents.is_empty() || self.navigator.usable())
        {
            let frame_number = ctx.cumulative_frame_nr();
            let previous = self.prepared_frame.get_or_insert(frame_number);
            let text_drawn = !self.use_native || self.active.is_none_or(|id| self.native.drawn(id));
            if frame_number > *previous && text_drawn {
                self.measured = true;
                let result = if self.startup_failed { b'2' } else { b'0' };
                if self.startup_failed
                    && let Some(message) = &self.status
                {
                    eprintln!("{message}");
                }
                if let Err(error) = lifecycle::notify(result) {
                    self.status = Some(format!("Cannot acknowledge startup: {error}"));
                }
                if std::env::var_os("GLIMPSE_MEASURE").is_some() {
                    eprintln!(
                        "FIRST_USABLE_FRAME_MS {:.3}",
                        self.started.elapsed().as_secs_f64() * 1000.
                    );
                }
                if self.wait_session && self.wait_paths.is_empty() {
                    ctx.request_repaint();
                }
            } else if frame_number <= *previous {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
                ctx.request_repaint();
            }
        }
    }
}

impl eframe::App for Glimpse {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _timing = crate::perf::Span::new("logic");
        if self.use_native {
            for (id, text) in self.native.changes() {
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                    doc.text = text;
                    doc.generation += 1;
                    doc.preview = false;
                    self.native.acknowledge(id, doc.generation);
                    self.io.send(Job::Recover(
                        id,
                        storage::Draft {
                            path: doc.disk.path.clone(),
                            text: doc.text.clone(),
                            base: doc.disk.bytes.clone(),
                            target: Some(doc.disk.target.clone()),
                            generation: doc.generation,
                        },
                    ));
                }
            }
        }
        if self.use_native {
            self.composing = self.native.composing();
        }
        self.receive(ctx);
        self.reconcile_destinations();
        while let Ok(batch) = self.completions.replies.try_recv() {
            if let Some((generation, _, prefix)) = &self.completion
                && *generation == batch.generation
            {
                for entry in batch.entries {
                    if name(&entry.path).starts_with(prefix) && self.completed_paths.len() < 32 {
                        self.completed_paths.push((entry.path, entry.directory));
                    }
                }
                self.completed_paths.sort();
                self.completed_paths.dedup();
                if batch.done
                    && batch.error.is_none()
                    && self.completed_paths.len() == 1
                    && let Some(text) = self.completed_paths[0].0.to_str()
                    && !text.chars().any(char::is_control)
                {
                    let (_, directory) = &self.completed_paths[0];
                    self.path_input = text.to_owned();
                    if *directory && !self.path_input.ends_with('/') {
                        self.path_input.push('/');
                    }
                    let id = egui::Id::unique("path_input");
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(self.path_input.chars().count()),
                            )));
                        state.store(ctx, id);
                    }
                    self.completed_paths.clear();
                    self.completion = None;
                    self.completions.retain(&[]);
                }
                if let Some(error) = batch.error {
                    self.status = Some(format!("Path completion: {error}"));
                }
                if self.completed_paths.len() >= 32 {
                    self.completions.retain(&[]);
                }
            }
        }

        for doc in &mut self.documents {
            if !doc.dirty()
                && !self.native.is_locked(doc.id)
                && !doc.saving
                && !self.composing
                && !ctx.input(|i| i.pointer.any_down())
                && doc
                    .external
                    .as_ref()
                    .is_some_and(|(disk, _)| disk.target == doc.disk.target)
                && let Some((disk, text)) = doc.external.take()
            {
                editor::replace(
                    ctx,
                    egui::Id::unique(("document", doc.id)),
                    &doc.text,
                    &text,
                );
                doc.text = text.clone();
                doc.saved_text = text;
                doc.disk = disk;
                doc.generation += 1;
                doc.conflict = None;
            }
            doc.metadata.update(&doc.text, doc.generation);
        }
        self.observer.update(
            self.documents
                .iter()
                .filter(|d| !d.disk.target.as_os_str().is_empty())
                .flat_map(|d| [d.disk.path.parent(), d.disk.target.parent()])
                .flatten()
                .map(Path::to_path_buf)
                .chain(self.navigator.requested_folders())
                .collect(),
        );
        let observed = self.observer.changed();
        let changed = matches!(observed, Some(Ok(())));
        if let Some(Err(error)) = observed {
            self.status = Some(format!(
                "File observation unavailable: {error}. Focus return and Save still revalidate."
            ));
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        if changed {
            self.navigator.refresh_visible();
        }
        if (focused && !self.was_focused) || changed {
            for doc in &self.documents {
                if !doc.disk.target.as_os_str().is_empty() && !doc.saving {
                    self.io
                        .send(Job::Check(doc.id, doc.disk.path.clone(), doc.generation));
                }
            }
        }
        self.was_focused = focused;
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            if self.read {
                self.markdown
                    .request(doc.id, doc.generation, &doc.text, &doc.disk.path);
            }
            let dark = ctx.theme() == egui::Theme::Dark;
            let request = (doc.id, doc.generation, dark);
            if self.settings.syntax && self.syntax_request != Some(request) {
                self.syntax
                    .request(doc.id, doc.generation, &doc.text, &doc.disk.path, dark);
                self.syntax_request = Some(request);
            }
        }
        if let Some(highlight) = self.syntax.take()
            && let Some(doc) = self
                .documents
                .iter_mut()
                .find(|d| d.id == highlight.id && d.generation == highlight.generation)
        {
            doc.highlight = Some(highlight);
        }

        while let Ok(result) = self.search.replies.try_recv() {
            if result.generation == self.search_id {
                self.hits = result.hits;
                self.search_state = result.state;
            }
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            // A close event can arrive in a logic-only pass while AppKit owns focus.
            // Schedule the confirmation pass even when no other input follows.
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let _timing = crate::perf::Span::new("ui");
        self.render(ui, frame);
    }
}
