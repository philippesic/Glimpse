mod completion;
mod conflict;
mod decision;
mod destination;
mod menus;
mod modal;
mod panes;
mod path;
mod platform;
mod preferences;
mod recovery;
mod theme;

use glimpse::directory::Entry;
use glimpse::{
    cli, directory,
    document::DiskFile,
    io::{IoWorker, Job, Reply, SaveDestination},
    lifecycle, observe, paths, search, storage, wake,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Root, Selectable, Sizable,
    button::Button,
    input::{Input, InputEvent, InputState, Position},
    text::TextView,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

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
                    app_id: Some("glimpse".into()),
                    window_min_size: Some(size(px(520.), px(400.))),
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
    resolved_through: u64,
    recovery_error: Option<(u64, String)>,
    error: Option<String>,
    failure_action: FailureAction,
    failed_destination: Option<(PathBuf, DestinationKind)>,
    unrecovered_save: Option<u64>,
    external: Option<(DiskFile, String)>,
    tab_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
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
    Recovery,
    Line,
    NewFile,
    NewFolder,
    Rename,
}
struct OpenIntent {
    preview: bool,
    position: Option<(usize, usize)>,
    interaction: u64,
}
struct CloseDecision {
    quit: bool,
    documents: Vec<u64>,
    saving: bool,
    finish: bool,
    validating: bool,
    discard: bool,
}
enum DecisionAction {
    TrashRecord(PathBuf),
    Conflict(Box<conflict::Snapshot>),
    Compare,
    Close,
    AbandonDestination(Box<SaveDestination>),
    ReplaceDestination(Box<SaveDestination>),
    UnrecoveredSave(Box<recovery::UnrecoveredSave>),
    RecoveryOptions(u64),
    FailureDetails(u64),
    Warnings,
    Notice,
}

#[derive(Clone, Copy, PartialEq)]
enum FailureAction {
    Save,
    Reload,
    Check,
    Choose(DestinationKind),
}

#[derive(Clone, Copy, PartialEq)]
enum DestinationKind {
    SaveAs,
    Copy,
    RecoveryCopy,
}

enum SettingsWrite {
    Preferences,
    Live,
    Pin,
    Visits(Vec<PathBuf>),
}

struct Shell {
    request: cli::Request,
    io: IoWorker,
    directories: directory::Directories,
    observer: observe::Observer,
    checks: observe::Checks,
    search: search::Search,
    syntax: glimpse::syntax::Syntax,
    search_id: u64,
    hits: Vec<search::Hit>,
    scope: Option<PathBuf>,
    current: PathBuf,
    history: Vec<PathBuf>,
    gesture: glimpse::gesture::Gesture,
    future: Vec<PathBuf>,
    nav_focus: FocusHandle,
    pane_state: Entity<panes::Divider>,
    navigation_only: bool,
    navigator_visible: bool,
    scrolls: HashMap<PathBuf, UniformListScrollHandle>,
    selection: Option<PathBuf>,
    selections: HashMap<PathBuf, PathBuf>,
    entries: Vec<Entry>,
    listing_id: u64,
    listing_done: bool,
    filter: String,
    documents: Vec<Document>,
    active: Option<u64>,
    tab_scroll: ScrollHandle,
    next_id: u64,
    open_token: u64,
    interaction: u64,
    path_request: Option<u64>,
    path_completion_token: u64,
    path_completions: Option<path::Completions>,
    opening: HashMap<u64, OpenIntent>,
    hit_versions: HashMap<u64, u64>,
    panel: Option<Panel>,
    query: Entity<InputState>,
    reading: bool,
    status: String,
    warnings: Vec<String>,
    omitted_warnings: u64,
    volumes: Option<glimpse::locations::Volumes>,
    settings: storage::Settings,
    settings_write: Option<SettingsWrite>,
    settings_loaded: bool,
    settings_reloading: bool,
    location_visits_pending: Vec<PathBuf>,
    preferences: Option<Entity<preferences::Preferences>>,
    preferences_subscription: Option<Subscription>,
    preferences_return: Option<FocusHandle>,
    subscriptions: Vec<Subscription>,
    decision: Option<Entity<decision::Decision>>,
    decision_action: Option<DecisionAction>,
    decision_subscription: Option<Subscription>,
    decision_return: Option<FocusHandle>,
    close_pending: Option<CloseDecision>,
    closings: glimpse::closing::Closings,
    quit_after_cleanup: bool,
    abandon_destination: Option<(u64, PathBuf, u64)>,
    destination_busy: bool,
    destination_document: Option<u64>,
    records: Vec<storage::Record>,
    recovery_usage: storage::RecoveryUsage,
    records_truncated: bool,
    importing: bool,
    ready: bool,
    startup_failed: bool,
    requested: Vec<(u64, PathBuf)>,
    finished: bool,
    validation_disks: HashMap<u64, DiskFile>,
    cleanup_texts: HashMap<u64, (u64, String)>,
    completed: HashSet<u64>,
    validated: HashSet<u64>,
    keep_after_cleanup: HashSet<u64>,
    finishing: glimpse::validation::Validations,
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
            if matches!(event, InputEvent::Change) && this.panel == Some(Panel::Path) {
                if this
                    .path_completions
                    .as_ref()
                    .is_some_and(|c| c.value != this.query.read(cx).value().as_str())
                {
                    this.path_completions = None;
                    this.path_completion_token += 1;
                }
                cx.notify();
            }
            if matches!(event, InputEvent::Change)
                && matches!(this.panel, Some(Panel::Files | Panel::Text))
            {
                this.search_id = this.search.cancel();
                this.hits.clear();
                cx.notify();
            }
            if matches!(event, InputEvent::Change)
                && matches!(this.panel, Some(Panel::Palette | Panel::Recovery))
            {
                cx.notify();
            }
            if matches!(event, InputEvent::Change) && this.panel == Some(Panel::Filter) {
                this.filter = this.query.read(cx).value().to_string();
                cx.notify();
            }
        });
        let shortcut_view = cx.weak_entity();
        let shortcut_window = window.window_handle();
        let shortcut_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == shortcut_window {
                let _ = shortcut_view.update(cx, |this: &mut Self, cx| {
                    this.interaction += 1;
                    this.shortcut(&event.keystroke, window, cx)
                });
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
            checks: observe::Checks::default(),
            search,
            syntax,
            current,
            history: vec![],
            gesture: glimpse::gesture::Gesture::default(),
            future: vec![],
            nav_focus: cx.focus_handle().tab_stop(true),
            pane_state: cx.new(panes::Divider::new),
            navigation_only: false,
            navigator_visible: true,
            scrolls: HashMap::new(),
            selection: None,
            selections: HashMap::new(),
            entries: vec![],
            listing_id: 0,
            listing_done: false,
            filter: String::new(),
            documents: vec![],
            active: None,
            tab_scroll: ScrollHandle::new(),
            next_id: 1,
            open_token: 0,
            interaction: 0,
            path_request: None,
            path_completion_token: 0,
            path_completions: None,
            opening: HashMap::new(),
            hit_versions: HashMap::new(),
            panel: None,
            query,
            reading: false,
            status: String::new(),
            warnings: Vec::new(),
            omitted_warnings: 0,
            volumes: None,
            settings: storage::Settings::default(),
            settings_write: None,
            settings_loaded: false,
            settings_reloading: true,
            location_visits_pending: Vec::new(),
            preferences: None,
            preferences_subscription: None,
            preferences_return: None,
            subscriptions: vec![subscription, shortcut_subscription],
            decision: None,
            decision_action: None,
            decision_subscription: None,
            decision_return: None,
            close_pending: None,
            closings: glimpse::closing::Closings::default(),
            quit_after_cleanup: false,
            abandon_destination: None,
            destination_busy: false,
            destination_document: None,
            records: vec![],
            recovery_usage: storage::RecoveryUsage::default(),
            records_truncated: false,
            importing: false,
            ready: false,
            startup_failed: false,
            requested: vec![],
            finished: false,
            validation_disks: HashMap::new(),
            cleanup_texts: HashMap::new(),
            completed: HashSet::new(),
            validated: HashSet::new(),
            keep_after_cleanup: HashSet::new(),
            finishing: glimpse::validation::Validations::default(),
            search_id: 0,
            hits: vec![],
        };
        window.focus(&this.nav_focus);
        this.refresh();
        this.subscriptions
            .push(cx.observe(&this.pane_state, |_, _, cx| cx.notify()));
        this.io.send(Job::LoadSettings);
        this.io.send(Job::Volumes);
        let files = this.request.files.clone();
        for (index, path) in files.into_iter().enumerate() {
            let position = (index == 0).then_some((this.request.line, this.request.column));
            this.open(path, false, position);
        }
        this
    }
    fn open_preferences(
        &mut self,
        settings: storage::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_write.is_some() || self.settings_reloading {
            self.status = "Wait for the current settings operation, then open Preferences".into();
            cx.notify();
            return;
        }
        if self.preferences.is_none() {
            self.preferences_return = window.focused(cx);
        }
        let view = cx.new(|cx| preferences::Preferences::new(settings, window, cx));
        self.preferences_subscription = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event, window, cx| match event {
                preferences::Event::Apply(settings) => {
                    let mut settings = settings.as_ref().clone();
                    settings.disk_base = this.settings.disk_base.clone();
                    settings.pins = this.settings.pins.clone();
                    settings.recents = this.settings.recents.clone();
                    settings.location_visits = this.settings.location_visits.clone();
                    if let Err(error) = this.queue_settings(settings, SettingsWrite::Preferences)
                        && let Some(view) = &this.preferences
                    {
                        view.update(cx, |view, cx| view.failed(error, cx));
                    }
                }
                preferences::Event::Cancel => this.close_preferences(window, cx),
                preferences::Event::Reload => {
                    this.settings_reloading = true;
                    this.io.send(Job::LoadSettings);
                }
            },
        ));
        self.preferences = Some(view);
        cx.notify();
    }

    fn close_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preferences.take().is_some() {
            self.preferences_subscription = None;
            if let Some(focus) = self.preferences_return.take() {
                window.focus(&focus);
            } else {
                self.focus(window, cx);
            }
        }
        self.flush_locations();
        cx.notify();
    }

    fn queue_settings(
        &mut self,
        settings: storage::Settings,
        purpose: SettingsWrite,
    ) -> Result<(), String> {
        if self.settings_write.is_some() || self.settings_reloading {
            return Err("A settings operation is in progress. Retry after it finishes.".into());
        }
        self.settings_write = Some(purpose);
        self.io.send(Job::Settings(settings));
        Ok(())
    }

    fn record_location(&mut self, path: PathBuf) {
        if self.request.pick || (self.settings_loaded && !self.settings.remember_locations) {
            return;
        }
        self.location_visits_pending.retain(|other| *other != path);
        self.location_visits_pending.push(path);
        if self.location_visits_pending.len() > 32 {
            self.location_visits_pending.remove(0);
        }
        self.flush_locations();
    }

    fn flush_locations(&mut self) {
        if !self.settings_loaded
            || self.settings_reloading
            || self.settings_write.is_some()
            || self.preferences.is_some()
            || self.settings.warning.is_some()
        {
            return;
        }
        if !self.settings.remember_locations {
            self.location_visits_pending.clear();
            return;
        }
        let visits = std::mem::take(&mut self.location_visits_pending);
        let mut settings = self.settings.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut changed = false;
        for path in &visits {
            changed |= settings.record_location(path, now);
        }
        if changed {
            let _ = self.queue_settings(settings, SettingsWrite::Visits(visits));
        }
    }

    fn apply_settings(
        &mut self,
        settings: storage::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings = settings;
        if let Some(warning) = &self.settings.warning {
            self.status = warning.clone();
        }
        theme::apply(&self.settings, window, cx);
        self.search_id = self.search.cancel();
        self.hits.clear();
        for doc in &self.documents {
            doc.input.update(cx, |input, cx| {
                input.set_soft_wrap(self.settings.wrap, window, cx);
                input.set_line_number(self.settings.line_numbers, window, cx);
                input.set_tab_size(
                    gpui_component::input::TabSize {
                        tab_size: self.settings.indent,
                        hard_tabs: self.settings.tabs,
                    },
                    cx,
                );
                input.set_auto_indent(self.settings.auto_indent, cx);
                if !self.settings.syntax {
                    input.set_external_highlights(vec![], cx);
                }
            });
        }
        self.refresh();
        self.focus(window, cx);
        cx.notify();
    }

    fn ask_decision(
        &mut self,
        action: DecisionAction,
        title: String,
        detail: String,
        actions: Vec<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.decision.is_some() {
            return;
        }
        self.decision_return = window.focused(cx);
        let view = cx.new(|cx| decision::Decision::new(title, detail, actions, window, cx));
        self.decision_subscription =
            Some(
                cx.subscribe_in(&view, window, |this, _, choice, window, cx| {
                    let action = this.decision_action.take();
                    this.decision = None;
                    this.decision_subscription = None;
                    if let Some(focus) = this.decision_return.take() {
                        window.focus(&focus);
                    }
                    match action {
                        Some(DecisionAction::Close) => this.command(
                            match choice.0 {
                                1 => "save-close",
                                2 => "discard-close",
                                _ => "cancel-close",
                            },
                            window,
                            cx,
                        ),
                        Some(DecisionAction::TrashRecord(path)) if choice.0 != 0 => {
                            this.io.send(Job::TrashRecord(path))
                        }
                        Some(DecisionAction::Conflict(snapshot)) if choice.0 != 0 => {
                            this.resolve_conflict(*snapshot, window, cx)
                        }
                        Some(DecisionAction::AbandonDestination(destination)) => {
                            if choice.0 == 0 {
                                this.cancel_destination();
                            } else {
                                this.abandon_destination = Some((
                                    destination.id,
                                    destination.path.clone(),
                                    destination.generation,
                                ));
                                this.authorize_destination(*destination, window, cx);
                            }
                        }
                        Some(DecisionAction::ReplaceDestination(destination)) => {
                            if choice.0 == 0 {
                                this.cancel_destination();
                            } else {
                                this.io.send(Job::SaveTo(*destination));
                            }
                        }
                        Some(DecisionAction::UnrecoveredSave(snapshot)) if choice.0 != 0 => {
                            this.save_unrecovered(*snapshot, cx)
                        }
                        Some(DecisionAction::RecoveryOptions(id))
                            if this.documents.iter().any(|doc| doc.id == id) =>
                        {
                            this.active = Some(id);
                            match choice.0 {
                                1 => this.retry_recovery(cx),
                                2 => this.save_as(DestinationKind::RecoveryCopy, window, cx),
                                3 => this.ask_unrecovered_save(window, cx),
                                _ => this.focus(window, cx),
                            }
                        }
                        Some(DecisionAction::FailureDetails(id))
                            if this.documents.iter().any(|doc| doc.id == id) =>
                        {
                            this.active = Some(id);
                            match choice.0 {
                                1 => this.retry_operation(window, cx),
                                2 => this.save_as(DestinationKind::Copy, window, cx),
                                _ => this.focus(window, cx),
                            }
                        }
                        Some(DecisionAction::Warnings) => match choice.0 {
                            1 => this.panel(Panel::Recovery, window, cx),
                            2 => {
                                this.warnings.clear();
                                this.omitted_warnings = 0;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                    cx.notify();
                }),
            );
        self.decision_action = Some(action);
        self.decision = Some(view);
        cx.notify();
    }

    fn narrow(&self, window: &Window) -> bool {
        window.viewport_size().width < px(760. * self.settings.ui_scale)
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
    fn check_document(&mut self, id: u64) {
        if let Some(doc) = self
            .documents
            .iter()
            .find(|d| d.id == id && !d.saving && !d.disk.target.as_os_str().is_empty())
        {
            let token = self.checks.begin(id, doc.generation);
            self.io.send(Job::Check(id, doc.disk.path.clone(), token));
        }
    }
    fn refresh(&mut self) {
        self.entries.clear();
        self.listing_done = false;
        self.listing_id = self.directories.request(self.current.clone()).unwrap_or(0);
        self.watch();
    }
    fn navigate(&mut self, path: PathBuf) {
        if path != self.current {
            self.history.push(self.current.clone());
            self.future.clear();
            self.change_location(path);
        }
    }
    fn change_location(&mut self, path: PathBuf) {
        if let Some(selection) = self.selection.take() {
            self.selections.insert(self.current.clone(), selection);
        }
        self.interaction += 1;
        self.path_completions = None;
        self.path_completion_token += 1;
        self.io.cancel_previews(self.open_token + 1);
        self.current = path;
        self.selection = self.selections.get(&self.current).cloned();
        self.filter.clear();
        self.hits.clear();
        self.search_id = self.search.cancel();
        self.refresh();
    }
    fn selected_path(&self) -> Option<PathBuf> {
        self.selection
            .as_ref()
            .filter(|path| {
                path.parent() == Some(self.current.as_path())
                    && self.entries.iter().any(|entry| entry.path == **path)
                    && (self.settings.show_hidden || !paths::name(path).starts_with('.'))
                    && paths::name(path)
                        .to_lowercase()
                        .contains(&self.filter.to_lowercase())
            })
            .cloned()
    }
    fn open(
        &mut self,
        path: PathBuf,
        preview: bool,
        position: Option<(usize, usize)>,
    ) -> Option<u64> {
        if self.destination_busy {
            self.status = "Finish the destination save before opening another file".into();
            return None;
        }
        self.open_token += 1;
        self.opening.insert(
            self.open_token,
            OpenIntent {
                preview,
                position,
                interaction: self.interaction,
            },
        );
        self.io.send(Job::Open(path, self.open_token, preview));
        Some(self.open_token)
    }
    fn add_document(
        &mut self,
        disk: DiskFile,
        text: String,
        intent: OpenIntent,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let OpenIntent {
            preview, position, ..
        } = intent;
        if !disk.target.as_os_str().is_empty()
            && let Some(parent) = disk.path.parent()
        {
            self.record_location(parent.to_owned());
        }
        if let Some(doc) = self.documents.iter_mut().find(|d| d.disk.same_file(&disk)) {
            if !preview {
                doc.preview = false;
            }
            if self.request.files.contains(&disk.path)
                && !self.requested.iter().any(|(_, path)| *path == disk.path)
            {
                self.requested.push((doc.id, disk.path.clone()));
            }
            let id = doc.id;
            if activate {
                self.active = Some(id);
                self.navigation_only = false;
                if let Some(position) = position {
                    self.position_document(id, position, true, window, cx);
                } else {
                    self.focus(window, cx);
                }
            }
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
            input.set_tab_size(
                gpui_component::input::TabSize {
                    tab_size: self.settings.indent,
                    hard_tabs: self.settings.tabs,
                },
                cx,
            );
            input.set_auto_indent(self.settings.auto_indent, cx);
            input
        });
        let subscription = cx.subscribe_in(&input, window, move |this, _, event, _, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            if let Some(doc) = this.documents.iter_mut().find(|d| d.id == id) {
                if this.finishing.invalidate_document(id) {
                    this.validation_disks.clear();
                    this.close_pending = None;
                    this.status =
                        "Completion cancelled because a requested document changed".into();
                }
                this.validated.remove(&id);
                this.completed.remove(&id);
                doc.generation += 1;
                if this.closings.contains(id) {
                    this.closings.forget(id);
                    this.cleanup_texts.remove(&id);
                    this.keep_after_cleanup.remove(&id);
                    this.quit_after_cleanup = false;
                    this.status = "Close cancelled because the document was edited".into();
                }
                glimpse::metrics::event("change", id, doc.generation);
                doc.preview = false;
                if !this.request.pick && !doc.dirty(cx) {
                    this.io.send(Job::ClearThrough(id, doc.generation));
                } else if !this.request.pick {
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
                    cx.theme().is_dark(),
                );
            }
            cx.notify();
        });
        if self.request.files.contains(&disk.path) {
            self.requested.push((id, disk.path.clone()));
        }
        let tab_focus = cx.focus_handle();
        let tab_subscription = cx.on_focus_in(&tab_focus, window, move |this, _, cx| {
            if let Some(index) = this.documents.iter().position(|doc| doc.id == id) {
                this.tab_scroll.scroll_to_item(index);
                cx.notify();
            }
        });
        self.documents.push(Document {
            id,
            disk,
            input: input.clone(),
            saved: text,
            generation: 0,
            preview,
            saving: false,
            restored: false,
            resolved_through: 0,
            recovery_error: None,
            error: None,
            failure_action: FailureAction::Save,
            failed_destination: None,
            unrecovered_save: None,
            external: None,
            tab_focus,
            _subscriptions: vec![subscription, tab_subscription],
        });
        self.tab_scroll.scroll_to_item(self.documents.len() - 1);
        self.syntax.request(
            id,
            0,
            &input.read(cx).value(),
            &disk_path(&self.documents, id),
            cx.theme().is_dark(),
        );
        self.watch();
        if activate {
            self.active = Some(id);
            self.reading = false;
            self.navigation_only = false;
        }
        if activate || position.is_some() {
            self.position_document(id, position.unwrap_or((1, 1)), activate, window, cx);
        }
    }

    fn position_document(
        &mut self,
        id: u64,
        (line, column): (usize, usize),
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self
            .documents
            .iter()
            .find(|doc| doc.id == id)
            .map(|doc| doc.input.clone())
        else {
            return;
        };
        let text = input.read(cx).value();
        let (position, clamped) = glimpse::editor::position(&text, line, column);
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
            input.set_cursor_position_unfocused(Position::new(row, column), cx);
            if focus {
                input.focus(window, cx);
            }
        });
        if clamped {
            self.status = "Requested position clamped to a text boundary".into();
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self
            .documents
            .iter()
            .position(|doc| Some(doc.id) == self.active)
        {
            self.tab_scroll.scroll_to_item(index);
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            self.syntax.request(
                doc.id,
                doc.generation,
                &doc.input.read(cx).value(),
                &doc.disk.path,
                cx.theme().is_dark(),
            );
            doc.input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.nav_focus);
        }
    }
    fn panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.path_completions = None;
        self.path_completion_token += 1;
        self.panel = Some(panel);
        let value = if panel == Panel::Path {
            paths::entry(&self.current)
        } else {
            String::new()
        };
        self.query.update(cx, |input, cx| {
            input.set_placeholder(
                match panel {
                    Panel::Path => "Absolute or relative path",
                    Panel::Filter => "Filter filenames",
                    Panel::Files => "Filename to find",
                    Panel::Text => "Text in saved files",
                    Panel::Palette => "Search commands",
                    Panel::Recovery => "Search recovery records",
                    Panel::Line => "Line number",
                    Panel::NewFile | Panel::NewFolder | Panel::Rename => "One filename",
                },
                window,
                cx,
            );
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
                if let Some(command) = glimpse::commands::COMMANDS.iter().find(|command| {
                    command.label.to_lowercase().contains(&query) || command.id.contains(&query)
                }) {
                    self.panel = None;
                    self.focus(window, cx);
                    self.command(command.id, window, cx);
                }
            }
            Some(Panel::Path) => match paths::parse_entry(&value, &self.current) {
                Ok(path) => {
                    if let Some(old) = self.path_request.take() {
                        self.opening.remove(&old);
                    }
                    self.path_request = self.open(path, false, None);
                }
                Err(error) => self.status = error,
            },
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
                if !paths::single_name(&value) {
                    self.status = "Enter one filename".into();
                } else if self.panel == Some(Panel::Rename) {
                    if let Some(source) = self.selected_path() {
                        if self.documents.iter().any(|doc| doc.disk.path == source) {
                            self.status = "Close the file before renaming".into();
                        } else {
                            let destination = source.with_file_name(&value);
                            self.io.send(Job::Rename(source, destination));
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
        let protected_focus = (self.preferences.is_some() || self.decision.is_some())
            .then(|| window.focused(cx))
            .flatten();
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
                Reply::PathCompleted(token, value, result) => {
                    self.path_completed(token, value, result, window, cx)
                }
                Reply::Volumes(result) => match result {
                    Ok(volumes) => self.volumes = Some(volumes),
                    Err(error) => self.status = format!("Mounted folders: {error}"),
                },
                Reply::OpenDirectory(path, token) => {
                    if let Some(intent) = self.opening.remove(&token) {
                        if !self.ready && self.request.files.contains(&path) {
                            self.startup_failed = true;
                            self.status = format!(
                                "Requested file became a directory: {}",
                                paths::display(&path)
                            );
                        } else if intent.interaction == self.interaction && token == self.open_token
                        {
                            if self.path_request == Some(token) {
                                self.path_request = None;
                                self.panel = None;
                            }
                            self.navigate(path);
                            self.navigation_only = true;
                            window.focus(&self.nav_focus);
                        }
                    }
                }
                Reply::Open(path, token, result) => {
                    if let Some(OpenIntent {
                        preview,
                        position,
                        interaction,
                    }) = self.opening.remove(&token)
                    {
                        let activate = interaction == self.interaction
                            && self.preferences.is_none()
                            && self.decision.is_none();
                        if preview && (!activate || token != self.open_token) {
                            continue;
                        }
                        let from_path = self.path_request == Some(token);
                        if from_path {
                            self.path_request = None;
                        }
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
                                if from_path && activate {
                                    self.panel = None;
                                }
                                self.add_document(
                                    disk,
                                    text,
                                    OpenIntent {
                                        preview,
                                        position,
                                        interaction,
                                    },
                                    activate,
                                    window,
                                    cx,
                                )
                            }
                            Err(e) => {
                                self.startup_failed |=
                                    !self.ready && self.request.files.contains(&path);
                                self.status = e;
                                if !from_path && activate {
                                    self.focus(window, cx);
                                }
                            }
                        }
                    }
                }
                Reply::Cleared(id, generation, result) => {
                    if let Some((pending_generation, text)) = self.cleanup_texts.get(&id)
                        && *pending_generation == generation
                        && let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == id)
                        && doc.input.read(cx).value().as_str() != text
                    {
                        doc.generation += 1;
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
                    let current = self
                        .documents
                        .iter()
                        .find(|d| d.id == id)
                        .map_or(generation, |d| d.generation);
                    match self.closings.acknowledge(id, generation, current, result) {
                        glimpse::closing::Outcome::Completed { discard } => {
                            self.cleanup_texts.remove(&id);
                            let requested =
                                self.requested.iter().any(|(requested, _)| *requested == id);
                            if discard
                                && requested
                                && self
                                    .documents
                                    .iter()
                                    .any(|doc| doc.id == id && doc.dirty(cx))
                                && self.request.wait
                                && !self.finished
                            {
                                let _ = lifecycle::notify(b'1');
                                self.finished = true;
                            }
                            if self.validated.remove(&id) {
                                self.completed.insert(id);
                            }
                            if !self.keep_after_cleanup.remove(&id) {
                                self.documents.retain(|d| d.id != id);
                                self.checks.invalidate(id);
                                if self.active == Some(id) {
                                    self.active = self.documents.last().map(|d| d.id);
                                }
                            }
                            self.complete_request();
                            self.focus(window, cx);
                            self.finish_close(cx);
                        }
                        glimpse::closing::Outcome::Changed => {
                            self.cleanup_texts.remove(&id);
                            self.keep_after_cleanup.remove(&id);
                            self.validated.remove(&id);
                            self.quit_after_cleanup = false;
                            self.status =
                                "Completion cancelled because the document changed".into();
                        }
                        glimpse::closing::Outcome::Failed(error) => {
                            self.status =
                                format!("Recovery cleanup failed; document kept open: {error}")
                        }
                        glimpse::closing::Outcome::Ignored => {}
                    }
                }
                Reply::Save(id, generation, text, result) => {
                    glimpse::metrics::event("save-reply", id, generation);
                    let succeeded = result.is_ok();
                    if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                        doc.saving = false;
                        let unrecovered = doc.unrecovered_save.take() == Some(generation);
                        match result {
                            Ok(disk) => {
                                doc.resolved_through = doc.resolved_through.max(generation);
                                doc.error = None;
                                doc.failure_action = FailureAction::Save;
                                doc.failed_destination = None;
                                if !unrecovered {
                                    doc.recovery_error = None;
                                }
                                doc.disk = disk;
                                doc.saved = text;
                                doc.restored = false;
                                doc.external = None;
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
                                } else {
                                    self.io.send(Job::ClearThrough(id, doc.generation));
                                }
                                self.io
                                    .send(Job::BindingApplied(id, generation, doc.generation));
                                self.status = match (unrecovered, doc.dirty(cx)) {
                                    (true, true) => {
                                        "Saved without recovery. Newer edits are still unsaved."
                                    }
                                    (true, false) => "Saved without recovery for this attempt",
                                    (false, true) => "Saved. Newer edits are still unsaved.",
                                    (false, false) => "Saved",
                                }
                                .into();
                            }
                            Err(error) => {
                                if error.starts_with("Recovery unavailable:") {
                                    doc.recovery_error = Some((generation, error.clone()));
                                }
                                doc.failed_destination = None;
                                doc.failure_action = FailureAction::Save;
                                doc.error = Some(error.clone());
                                self.status = error;
                            }
                        }
                    }
                    self.check_document(id);
                    if self.close_pending.as_ref().is_some_and(|d| d.saving) {
                        if succeeded {
                            self.continue_close(window, cx);
                        } else {
                            self.close_pending = None;
                        }
                    }
                }
                Reply::SavedTo(destination, result) => {
                    let succeeded = result.is_ok();
                    let id = destination.id;
                    self.checks.invalidate(id);
                    self.destination_busy = false;
                    self.destination_document = None;
                    match result {
                        Ok(disk) => {
                            if let Some(doc) = self
                                .documents
                                .iter_mut()
                                .find(|doc| doc.id == destination.id)
                            {
                                if !destination.copy || doc.failure_action == FailureAction::Save {
                                    doc.error = None;
                                    doc.failed_destination = None;
                                }
                                if !destination.recovery_copy {
                                    doc.recovery_error = None;
                                }
                            }
                            if !destination.copy
                                && let Some(doc) =
                                    self.documents.iter_mut().find(|d| d.id == destination.id)
                            {
                                doc.resolved_through =
                                    doc.resolved_through.max(destination.generation);
                                doc.disk = disk;
                                doc.external = None;
                                doc.saved = destination.text;
                                doc.restored = false;
                                doc.generation += 1;
                                if doc.dirty(cx) {
                                    self.io.send(Job::Recover(
                                        doc.id,
                                        storage::Draft {
                                            path: doc.disk.path.clone(),
                                            target: Some(doc.disk.target.clone()),
                                            text: doc.input.read(cx).value().to_string(),
                                            base: doc.disk.bytes.clone(),
                                            generation: doc.generation,
                                        },
                                    ));
                                } else {
                                    self.io.send(Job::ClearThrough(doc.id, doc.generation));
                                }
                                self.io.send(Job::BindingApplied(
                                    doc.id,
                                    destination.generation,
                                    doc.generation,
                                ));
                            }
                            if !destination.copy
                                && self.abandon_destination.as_ref().is_some_and(
                                    |(id, path, generation)| {
                                        *id == destination.id
                                            && *path == destination.path
                                            && *generation == destination.generation
                                    },
                                )
                                && self.request.wait
                                && !self.finished
                            {
                                let _ = lifecycle::notify(b'1');
                                self.finished = true;
                                self.status =
                                    "Saved As. The original CLI destination was abandoned.".into();
                            } else {
                                self.status = if destination.recovery_copy {
                                    format!(
                                        "Recovery copy saved at {}. The original file is unchanged.",
                                        paths::display(&destination.path)
                                    )
                                } else if destination.copy {
                                    format!(
                                        "Copy saved at {}. The original file is unchanged.",
                                        paths::display(&destination.path)
                                    )
                                } else if self
                                    .documents
                                    .iter()
                                    .any(|doc| doc.id == destination.id && doc.dirty(cx))
                                {
                                    "Saved. Newer edits are still unsaved.".into()
                                } else {
                                    "Saved".into()
                                };
                            }
                        }
                        Err(error) => self.destination_failed(&destination, error),
                    }
                    self.abandon_destination = None;
                    if self.close_pending.as_ref().is_some_and(|d| d.saving) {
                        if succeeded {
                            self.continue_close(window, cx);
                        } else {
                            self.close_pending = None;
                        }
                    }
                }
                Reply::Prepared(mut destination, result) => match result {
                    Ok(expected) => {
                        let duplicate = self.documents.iter().any(|doc| {
                            doc.id != destination.id
                                && (doc.disk.path == destination.path
                                    || expected
                                        .as_ref()
                                        .is_some_and(|disk| doc.disk.same_file(disk)))
                        });
                        if duplicate {
                            self.destination_failed(
                                &destination,
                                "Close the destination’s open tab before replacing it".into(),
                            );
                        } else {
                            destination.expected = expected;
                            self.confirm_destination(*destination, window, cx);
                        }
                    }
                    Err(error) => {
                        self.destination_failed(&destination, error);
                    }
                },
                Reply::Checked(id, token, result) => {
                    let generation = self
                        .documents
                        .iter()
                        .find(|d| d.id == id)
                        .map(|d| d.generation);
                    if generation
                        .is_some_and(|generation| self.checks.accept(id, token, generation))
                        && let Some(doc) =
                            self.documents.iter_mut().find(|d| d.id == id && !d.saving)
                    {
                        if result.is_ok() && doc.failure_action == FailureAction::Check {
                            doc.error = None;
                            doc.failure_action = FailureAction::Save;
                            self.status = "File access restored".into();
                        }
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
                            Err(error) => {
                                doc.error = Some(error.clone());
                                doc.failure_action = FailureAction::Check;
                                doc.failed_destination = None;
                                self.status = format!("File unavailable: {error}");
                            }
                        }
                    }
                }
                Reply::Reloaded(reload, result) => self.reloaded(*reload, result, window, cx),
                Reply::SettingsLoaded(result) => {
                    self.settings_reloading = false;
                    self.settings_loaded = true;
                    match result {
                        Ok(settings) => {
                            self.apply_settings(settings, window, cx);
                            if self.preferences.is_some() {
                                self.open_preferences(self.settings.clone(), window, cx);
                                self.status = "Preferences reloaded".into();
                            }
                        }
                        Err(e) => {
                            if let Some(preferences) = &self.preferences {
                                preferences.update(cx, |view, cx| view.failed(e.clone(), cx));
                            }
                            self.settings.warning = Some(e.clone());
                            self.status = e;
                        }
                    }
                    self.flush_locations();
                }
                Reply::SettingsSaved(result) => {
                    let Some(purpose) = self.settings_write.take() else {
                        continue;
                    };
                    match result {
                        Ok(settings) => {
                            match purpose {
                                SettingsWrite::Preferences => {
                                    self.apply_settings(settings, window, cx);
                                    self.close_preferences(window, cx);
                                    self.status = "Preferences saved".into();
                                }
                                SettingsWrite::Live => self.apply_settings(settings, window, cx),
                                SettingsWrite::Pin => {
                                    self.settings = settings;
                                    self.status = "Pinned locations saved".into();
                                }
                                SettingsWrite::Visits(_) => self.settings = settings,
                            }
                            self.flush_locations();
                        }
                        Err(error) => {
                            match purpose {
                                SettingsWrite::Preferences => {
                                    if let Some(view) = &self.preferences {
                                        view.update(cx, |view, cx| view.failed(error.clone(), cx));
                                    }
                                }
                                SettingsWrite::Visits(visits) => {
                                    let mut pending = visits;
                                    for path in std::mem::take(&mut self.location_visits_pending) {
                                        pending.retain(|older| *older != path);
                                        pending.push(path);
                                    }
                                    if pending.len() > 32 {
                                        pending.drain(..pending.len() - 32);
                                    }
                                    self.location_visits_pending = pending;
                                }
                                _ => {}
                            }
                            self.status = format!("Settings not saved: {error}");
                        }
                    }
                }
                Reply::Records(result) => match result {
                    Ok(inventory) => {
                        self.recovery_usage = inventory.usage;
                        self.records_truncated = inventory.records_truncated;
                        self.records = inventory.records;
                        self.status = format!("{} recovery/history records", self.records.len());
                    }
                    Err(e) => self.status = e,
                },
                Reply::Imported(import_id, result) => {
                    self.importing = false;
                    match result {
                        Ok((draft, mut disk)) => {
                            let duplicate =
                                self.documents.iter().any(|doc| doc.disk.same_file(&disk));
                            if duplicate {
                                let mut copy = DiskFile::draft();
                                copy.path = draft.path.clone();
                                copy.bytes = draft.base.clone();
                                copy.bom = disk.bom;
                                disk = copy;
                            }
                            let next_id = self.next_id;
                            self.next_id = import_id;
                            self.add_document(
                                disk,
                                draft.text,
                                OpenIntent {
                                    preview: false,
                                    position: None,
                                    interaction: self.interaction,
                                },
                                true,
                                window,
                                cx,
                            );
                            self.next_id = self.next_id.max(next_id);
                            self.requested.retain(|(id, _)| *id != import_id);
                            if let Some(doc) =
                                self.documents.iter_mut().find(|doc| doc.id == import_id)
                            {
                                doc.restored = true;
                                doc.generation = draft.generation + 1;
                                self.io.send(Job::Recover(
                                    import_id,
                                    storage::Draft {
                                        path: doc.disk.path.clone(),
                                        target: Some(doc.disk.target.clone()),
                                        text: doc.input.read(cx).value().to_string(),
                                        base: doc.disk.bytes.clone(),
                                        generation: doc.generation,
                                    },
                                ));
                            }
                            self.panel = None;
                            self.status = if duplicate {
                                "Restored a separate copy because the original is already open. Use Save As.".into()
                            } else {
                                "Recovered local text. The original file has not been changed."
                                    .into()
                            };
                            self.check_document(import_id);
                        }
                        Err(error) => self.status = error,
                    }
                    self.io.send(Job::Inventory);
                }
                Reply::RecordTrashed(result) => {
                    self.status = match result {
                        Ok(()) => "Recovery record moved to Trash".into(),
                        Err(error) => error,
                    };
                    self.io.send(Job::Inventory);
                }
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
                Reply::Finished(id, generation, attempt, disk, result) => {
                    self.finished_validation((id, generation, attempt, disk), result, cx);
                }
                Reply::Recovered(id, generation, result) => {
                    if let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == id) {
                        match result {
                            Ok(()) => {
                                if generation > doc.resolved_through
                                    && doc
                                        .recovery_error
                                        .as_ref()
                                        .is_some_and(|(failed, _)| generation >= *failed)
                                {
                                    doc.recovery_error = None;
                                    self.status = "Recovery storage is available".into();
                                    if doc.failed_destination.is_none()
                                        && doc.error.as_ref().is_some_and(|error| {
                                            error.starts_with("Recovery unavailable:")
                                        })
                                    {
                                        doc.error = None;
                                    }
                                }
                                if !doc.dirty(cx) {
                                    self.io.send(Job::ClearThrough(id, generation));
                                }
                            }
                            Err(error) if generation > doc.resolved_through => {
                                doc.recovery_error = Some((generation, error))
                            }
                            Err(_) => {}
                        }
                    }
                }
                Reply::Done(Err(error)) => self.status = error,
                Reply::HistoryWarning(error) if !self.warnings.contains(&error) => {
                    if self.warnings.len() == 32 {
                        self.warnings.remove(0);
                        self.omitted_warnings = self.omitted_warnings.saturating_add(1);
                    }
                    self.warnings.push(error);
                }
                Reply::Changed(result) => match result {
                    Ok(()) => {
                        self.refresh();
                        window.focus(&self.nav_focus);
                    }
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
                    let ids: Vec<_> = self.documents.iter().map(|doc| doc.id).collect();
                    for id in ids {
                        self.check_document(id);
                    }
                    self.refresh();
                }
                Err(e) => self.status = e,
            }
        }
        if let Some(highlight) = self.syntax.take()
            && self.settings.syntax
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
                    let color = glimpse::syntax::readable_rgb(span.rgb, theme::canvas(cx));
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
        }
        if (self.preferences.is_some() || self.decision.is_some())
            && let Some(focus) = protected_focus
        {
            window.focus(&focus);
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        glimpse::metrics::event("save-command", self.active.unwrap_or(0), 0);
        if self.request.pick {
            return;
        }
        self.cancel_cleanup_for_active(window, cx);
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| Some(d.id) == self.active)
        {
            if doc.saving || (self.destination_busy && self.destination_document == Some(doc.id)) {
                self.status = "A save for this document is already pending".into();
                return;
            }
            if doc.disk.target.as_os_str().is_empty() {
                self.save_as(DestinationKind::SaveAs, window, cx);
                return;
            }
            if doc.external.is_some() {
                self.status =
                    "Resolve the external change before saving. Save Copy preserves local edits."
                        .into();
                return;
            }
            glimpse::metrics::event("save-queued", doc.id, doc.generation);
            self.checks.invalidate(doc.id);
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
    fn save_as(&mut self, kind: DestinationKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.request.pick || self.destination_busy {
            return;
        }
        self.cancel_cleanup_for_active(window, cx);
        let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) else {
            return;
        };
        let copy = kind != DestinationKind::SaveAs;
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
            recovery_copy: kind == DestinationKind::RecoveryCopy,
        };
        let previous = doc
            .failed_destination
            .as_ref()
            .filter(|(_, previous)| *previous == kind)
            .map(|(path, _)| path);
        let folder = previous
            .and_then(|path| path.parent())
            .unwrap_or(&self.current);
        let suggested = paths::name(previous.unwrap_or(&doc.disk.path));
        let prompt = cx.prompt_for_new_path(folder, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(path)) => {
                        if this.documents.iter().any(|doc| doc.id == destination.id) {
                            destination.path = path;
                            this.destination_busy = true;
                            this.destination_document = Some(destination.id);
                            this.io.send(Job::Prepare(destination));
                        } else {
                            this.status = "The document was closed; nothing was saved".into();
                        }
                    }
                    Ok(None) => {
                        this.close_pending = None;
                        this.status = "Save cancelled. The document remains open.".into();
                    }
                    Err(error) => {
                        this.close_pending = None;
                        let message = format!("Destination chooser failed: {error}");
                        if let Some(doc) = this
                            .documents
                            .iter_mut()
                            .find(|doc| doc.id == destination.id)
                        {
                            doc.error = Some(message.clone());
                            doc.failure_action = FailureAction::Choose(kind);
                        }
                        this.status = message;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn close(&mut self, quit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.decision.is_some() {
            return;
        }
        if let Some(preferences) = &self.preferences {
            if preferences.read(cx).busy() {
                self.status = "Wait for the settings operation to finish".into();
            } else {
                self.close_preferences(window, cx);
            }
            return;
        }
        if self.io.pending_mutations() > 0
            || self.destination_busy
            || !self.closings.is_empty()
            || self.finishing.pending()
        {
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
            finish: false,
            validating: false,
            discard: false,
        });
        self.continue_close(window, cx);
    }
    fn continue_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(decision) = &self.close_pending else {
            return;
        };
        if decision.validating {
            return;
        }
        let dirty = self
            .documents
            .iter()
            .find(|doc| decision.documents.contains(&doc.id) && doc.dirty(cx))
            .map(|doc| doc.id);
        if let Some(id) = dirty {
            if decision.saving {
                if self
                    .documents
                    .iter()
                    .any(|doc| doc.id == id && doc.external.is_some())
                {
                    self.close_pending = None;
                    self.status =
                        "Resolve the external change before saving and completing this decision"
                            .into();
                    cx.notify();
                    return;
                }
                self.active = Some(id);
                self.save(window, cx);
            } else if self.decision.is_none() {
                let names = self
                    .documents
                    .iter()
                    .filter(|doc| decision.documents.contains(&doc.id) && doc.dirty(cx))
                    .map(|doc| paths::display(&doc.disk.path))
                    .collect::<Vec<_>>()
                    .join("\n");
                let finish = decision.finish;
                self.ask_decision(DecisionAction::Close, if finish { "Finish Editing?" } else { "Close with unsaved changes?" }.into(),
                    format!("Unsaved documents:\n{names}\nSave writes these documents before continuing. Discard closes these documents and retires their automatic drafts. Cancel keeps them open."),
                    vec!["Cancel", if finish { "Save and Finish" } else { "Save and Close" }, "Discard"], window, cx);
            }
        } else {
            self.commit_close(false, cx);
        }
        cx.notify();
    }
    fn commit_close(&mut self, discard: bool, cx: &mut Context<Self>) {
        if let Some(decision) = self.close_pending.as_mut() {
            decision.discard = discard;
        }
        if self.validate_before_cleanup(cx) {
            return;
        }
        let Some(decision) = self.close_pending.take() else {
            return;
        };
        self.quit_after_cleanup = decision.quit;
        for doc in &self.documents {
            if decision.documents.contains(&doc.id)
                && self.closings.begin(doc.id, doc.generation, discard)
            {
                if decision.finish && !discard {
                    self.keep_after_cleanup.insert(doc.id);
                }
                self.cleanup_texts.insert(
                    doc.id,
                    (doc.generation, doc.input.read(cx).value().to_string()),
                );
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
    fn shortcut(&mut self, key: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel == Some(Panel::Path)
            && self.preferences.is_none()
            && self.decision.is_none()
            && self.query.focus_handle(cx).is_focused(window)
            && key.key == "tab"
            && !key.modifiers.control
            && !key.modifiers.platform
        {
            self.complete_path(key.modifiers.shift, window, cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
        if self.decision.is_some()
            || self.preferences.is_some()
            || (!key.modifiers.control && !key.modifiers.platform)
        {
            return;
        }
        let bindings = glimpse::commands::bindings(&self.settings.keymaps).unwrap_or_default();
        let command = bindings
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
                    && platform::shortcut(&binding.modifiers, &key.modifiers)
            })
            .and_then(|binding| {
                glimpse::commands::COMMANDS
                    .iter()
                    .find(|command| command.action == binding.action)
            });
        if let Some(command) = command {
            self.command(command.id, window, cx);
            window.prevent_default();
            cx.stop_propagation();
        } else if key.key.eq_ignore_ascii_case("f")
            && platform::shortcut(
                &glimpse::commands::Modifiers {
                    command: true,
                    ..Default::default()
                },
                &key.modifiers,
            )
        {
            window.prevent_default();
            cx.stop_propagation();
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
                    | "pin"
                    | "indent"
                    | "outdent"
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
            "cancel" => {
                if self.close_pending.is_some() || !self.closings.is_empty() {
                    self.command("cancel-close", window, cx);
                } else {
                    self.command("dismiss", window, cx);
                }
            }
            "named-file" => self.panel(Panel::NewFile, window, cx),
            "new-folder" => self.panel(Panel::NewFolder, window, cx),
            "rename" => self.panel(Panel::Rename, window, cx),
            "navigator" => {
                self.navigation_only = true;
                self.navigator_visible = true;
                window.focus(&self.nav_focus);
            }
            "toggle-navigator" => {
                if self.narrow(window) {
                    self.navigation_only = !self.navigation_only;
                    if self.navigation_only {
                        window.focus(&self.nav_focus);
                    } else {
                        self.focus(window, cx);
                    }
                } else {
                    self.navigator_visible = !self.navigator_visible;
                }
            }
            "find" => {
                self.focus(window, cx);
                window.dispatch_action(Box::new(gpui_component::input::Search), cx);
            }
            "submit" => self.submit(window, cx),
            "dismiss" => {
                if let Some(token) = self.path_request.take() {
                    self.opening.remove(&token);
                }
                if self.panel == Some(Panel::Filter) {
                    self.filter.clear();
                }
                self.panel = None;
                self.hits.clear();
                self.search_id = self.search.cancel();
                self.focus(window, cx);
            }
            "cancel-close" | "cancel-closing" => {
                self.finishing.invalidate();
                self.close_pending = None;
                self.keep_after_cleanup.clear();
                self.cleanup_texts.clear();
                self.validation_disks.clear();
                self.validated.clear();
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
            "cancel-destination" => self.cancel_destination(),
            "save-settings" => {
                if let Err(error) = self.queue_settings(self.settings.clone(), SettingsWrite::Live)
                {
                    self.status = error;
                }
            }
            "wrap" | "numbers" => {
                let mut settings = self.settings.clone();
                if command == "wrap" {
                    settings.wrap = !settings.wrap;
                } else {
                    settings.line_numbers = !settings.line_numbers;
                }
                if let Err(error) = self.queue_settings(settings, SettingsWrite::Live) {
                    self.status = error;
                }
            }
            "pin" => {
                let mut settings = self.settings.clone();
                if settings.pins.contains(&self.current) {
                    settings.pins.retain(|path| *path != self.current);
                } else {
                    settings.pins.push(self.current.clone());
                }
                if let Err(error) = self.queue_settings(settings, SettingsWrite::Pin) {
                    self.status = error;
                }
            }

            "save" => self.save(window, cx),
            "retry-operation" => self.retry_operation(window, cx),
            "failure-details" => self.failure_details(window, cx),
            "warnings" => {
                let mut detail = self.warnings.join("\n\n");
                if self.omitted_warnings > 0 {
                    detail.push_str(&format!(
                        "\n{} earlier warnings are outside this list.",
                        self.omitted_warnings
                    ));
                }
                self.ask_decision(
                    DecisionAction::Warnings,
                    "Recovery warning".into(),
                    detail,
                    vec!["Close", "Open Recovery", "Acknowledge"],
                    window,
                    cx,
                );
            }
            "discard-close" => self.commit_close(true, cx),
            "save-close" => {
                if let Some(decision) = self.close_pending.as_mut() {
                    decision.saving = true;
                }
                self.continue_close(window, cx);
            }
            "retry-close" | "retry-cleanup" => {
                for (id, generation) in self.closings.retry() {
                    self.io.send(Job::ClearThrough(id, generation));
                }
            }
            "save-as" => self.save_as(DestinationKind::SaveAs, window, cx),
            "save-copy" => self.save_as(DestinationKind::Copy, window, cx),
            "recovery-copy" => self.save_as(DestinationKind::RecoveryCopy, window, cx),
            "recovery-options" => self.recovery_options(window, cx),
            "retry-recovery" => self.retry_recovery(cx),
            "save-unrecovered" => self.ask_unrecovered_save(window, cx),
            "new" if !self.request.pick => self.add_document(
                DiskFile::draft(),
                String::new(),
                OpenIntent {
                    preview: false,
                    position: None,
                    interaction: self.interaction,
                },
                true,
                window,
                cx,
            ),
            "open" | "folder" => {
                let prompt = cx.prompt_for_paths(PathPromptOptions {
                    files: command == "open",
                    directories: true,
                    multiple: command == "open",
                    prompt: None,
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = prompt
                        .await
                        .map_err(|error| error.to_string())
                        .and_then(|result| result.map_err(|error| error.to_string()));
                    let _ = this.update_in(cx, |this, window, cx| {
                        match result {
                            Ok(Some(paths)) => {
                                for path in paths {
                                    this.open(path, false, None);
                                }
                            }
                            Ok(None) => this.status = "Open cancelled".into(),
                            Err(error) => this.ask_decision(
                                DecisionAction::Notice,
                                "File chooser unavailable".into(),
                                format!("{error}\nTry Open again after the desktop file chooser is available."),
                                vec!["Close"],
                                window,
                                cx,
                            ),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            "path" => self.panel(Panel::Path, window, cx),
            "filter" => self.panel(Panel::Filter, window, cx),
            "find-files" => self.panel(Panel::Files, window, cx),
            "find-text" => self.panel(Panel::Text, window, cx),
            "commands" => self.panel(Panel::Palette, window, cx),
            "settings" => self.open_preferences(self.settings.clone(), window, cx),
            "recovery" => self.panel(Panel::Recovery, window, cx),
            "refresh-recovery" => {
                self.io.send(Job::Inventory);
            }
            "line" => self.panel(Panel::Line, window, cx),
            "parent" => {
                if let Some(path) = self.current.parent().map(|p| p.to_owned()) {
                    self.navigate(path);
                }
            }
            "home" => self.navigate(std::env::home_dir().unwrap()),
            "back" => {
                if let Some(path) = self.history.pop() {
                    self.future.push(self.current.clone());
                    self.change_location(path);
                }
            }
            "forward" => {
                if let Some(path) = self.future.pop() {
                    self.history.push(self.current.clone());
                    self.change_location(path);
                }
            }
            "refresh" => self.refresh(),
            "refresh-volumes" => {
                self.io.send(Job::Volumes);
                self.status = "Refreshing mounted folders; reopen Locations for the result".into();
            }
            "indent" | "outdent" => {
                self.focus(window, cx);
                let action: Box<dyn Action> = if command == "indent" {
                    Box::new(gpui_component::input::Indent)
                } else {
                    Box::new(gpui_component::input::Outdent)
                };
                window.dispatch_action(action, cx);
            }
            "read" => self.reading = !self.reading,
            "keep-tab" => {
                if let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|doc| Some(doc.id) == self.active)
                {
                    doc.preview = false;
                }
            }
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
                self.finishing.invalidate();
                self.close_pending = None;
                let _ = lifecycle::notify(b'1');
                self.finished = true;
                self.status = "CLI request aborted".into();
            }
            "finish" => self.finish_requested(window, cx),
            "choose" if self.request.pick => {
                let path = if self.request.directory {
                    self.current.clone()
                } else if let Some(path) = self.selected_path() {
                    path
                } else {
                    return;
                };
                self.io.send(Job::Choose(path, self.request.directory));
            }
            "trash" if !self.request.pick => {
                if let Some(path) = self.selected_path() {
                    if self.documents.iter().any(|d| d.disk.path == path) {
                        self.status = "Close this file before moving it to Trash".into();
                    } else {
                        self.io.send(Job::Trash(path));
                    }
                }
            }
            "reload" | "overwrite" | "compare" => self.conflict_action(command, window, cx),
            "keep-editing" => self.focus(window, cx),
            "external" | "finder" => {
                if let Some(path) = self.selected_path() {
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
            "go-scope" => {
                if let Some(scope) = self.scope.clone() {
                    self.navigate(scope);
                }
            }
            "reveal" => {
                if let Some(path) = self
                    .documents
                    .iter()
                    .find(|doc| {
                        Some(doc.id) == self.active && !doc.disk.target.as_os_str().is_empty()
                    })
                    .map(|doc| doc.disk.path.clone())
                {
                    if let Some(parent) = path.parent() {
                        self.navigate(parent.to_owned());
                    }
                    self.selection = Some(path);
                    window.focus(&self.nav_focus);
                }
            }
            "copy-path" | "copy-relative" => {
                let path = self.selected_path().unwrap_or_else(|| self.current.clone());
                let scope = self.scope.as_ref().unwrap_or(&self.current);
                let value = if command == "copy-relative" {
                    path.strip_prefix(scope)
                        .map_err(|_| "Selected path is outside the search folder")
                } else {
                    Ok(path.as_path())
                };
                match value {
                    Ok(path) => {
                        if let Some(value) = path.to_str() {
                            cx.write_to_clipboard(ClipboardItem::new_string(value.to_owned()));
                        } else {
                            self.status = "This path cannot be copied as UTF-8 text. Use the CLI picker for a lossless path.".into();
                        }
                    }
                    Err(error) => self.status = error.into(),
                }
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
    ) -> Button {
        self.command_button(command, label, command, cx)
    }
    fn command_button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        command: &'static str,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id)
            .label(label)
            .small()
            .compact()
            .outline()
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
        let narrow = self.narrow(window);
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .p(px(8.))
            .bg(cx.theme().muted);
        if narrow {
            toolbar = toolbar
                .child(self.button(
                    if self.navigation_only && self.active.is_some() {
                        "Edit"
                    } else {
                        "Browse"
                    },
                    "toggle-navigator",
                    cx,
                ))
                .child(self.locations_menu(cx))
                .child(self.main_menu(cx));
        } else {
            toolbar = toolbar
                .child(self.main_menu(cx))
                .child(self.button("‹", "back", cx))
                .child(self.button("›", "forward", cx))
                .child(self.button("↑", "parent", cx))
                .child(
                    self.button("Location", "path", cx)
                        .tooltip(paths::display(&self.current)),
                )
                .child(self.locations_menu(cx))
                .child(self.button("Navigator", "toggle-navigator", cx))
                .child(self.button("Open", "open", cx))
                .child(self.button("Filter", "filter", cx))
                .child(self.button("Files", "find-files", cx))
                .child(self.button("Text", "find-text", cx))
                .child(self.button("Commands", "commands", cx));
        }
        if self.request.pick {
            toolbar = toolbar
                .child(self.button("Choose", "choose", cx))
                .child(self.button("Cancel", "cancel", cx));
        }
        let mut root = div()
            .size_full()
            .relative()
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.interaction += 1;
                if this.preferences.is_none()
                    && this.decision.is_none()
                    && let MouseButton::Navigate(direction) = event.button
                {
                    this.command(
                        if direction == NavigationDirection::Back {
                            "back"
                        } else {
                            "forward"
                        },
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            .can_drop({
                let enabled = self.preferences.is_none() && self.decision.is_none();
                move |_, _, _| enabled
            })
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.interaction += 1;
                for path in paths.paths().iter().take(64) {
                    this.open(path.clone(), false, None);
                }
                if paths.paths().len() > 64 {
                    this.status =
                        "Requested the first 64 dropped paths; drop the remaining paths separately"
                            .into();
                }
                cx.notify();
            }))
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(14. * self.settings.ui_scale))
            .child(toolbar)
            .child(self.path_bar(window, cx));
        if self.panel == Some(Panel::Path) {
            root = root.child(self.path_matches(cx));
        }
        if let Some(panel) = self.panel.filter(|panel| *panel != Panel::Path) {
            let title = match panel {
                Panel::Path => "Location",
                Panel::Filter => "Filter this folder",
                Panel::Files => "Find files",
                Panel::Text => "Find saved text",
                Panel::Palette => "Commands",
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
                    .child(Input::new(&self.query).flex_1().min_w_0())
                    .when(!matches!(panel, Panel::Recovery | Panel::Filter), |row| {
                        row.child(self.button("Apply", "submit", cx))
                    })
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
                                    this.open(path.clone(), false, Some((line, 1)));
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
                                .filter(|c| {
                                    c.label.to_lowercase().contains(&query) || c.id.contains(&query)
                                })
                                .map(|command| {
                                    let key = self
                                        .settings
                                        .keymaps
                                        .get(command.id)
                                        .map(String::as_str)
                                        .unwrap_or(command.shortcut);
                                    let label = if key.is_empty() {
                                        command.label.to_owned()
                                    } else {
                                        format!(
                                            "{}  {}",
                                            command.label,
                                            platform::shortcut_label(key)
                                        )
                                    };
                                    self.command_button(
                                        SharedString::from(format!("palette-{}", command.id)),
                                        label,
                                        command.id,
                                        cx,
                                    )
                                }),
                        ),
                );
            }
            if panel == Panel::Recovery {
                root = root.child(self.recovery_view(cx));
            }
        }

        if self.finishing.pending() {
            root = root.child(
                div()
                    .p_2()
                    .child("Validating requested destinations…")
                    .child(self.button("Cancel completion", "cancel-close", cx)),
            );
        }
        if !self.closings.is_empty() {
            root = root.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child("Resolving recovery before completing the decision")
                    .child(self.button("Retry cleanup", "retry-close", cx))
                    .child(self.button("Keep open", "cancel-close", cx)),
            );
        }

        let query = self.filter.to_lowercase();
        let visible: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                (self.settings.show_hidden || !paths::name(&entry.path).starts_with('.'))
                    && paths::name(&entry.path).to_lowercase().contains(&query)
            })
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
            .w_full()
            .h_full()
            .bg(cx.theme().sidebar)
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                if this.preferences.is_some() || this.decision.is_some() {
                    return;
                }
                let delta = event.delta.pixel_delta(px(24.));
                let phase = if !event.delta.precise() {
                    glimpse::gesture::Phase::Wheel
                } else {
                    match event.touch_phase {
                        TouchPhase::Started => glimpse::gesture::Phase::Started,
                        TouchPhase::Moved => glimpse::gesture::Phase::Moved,
                        TouchPhase::Ended => glimpse::gesture::Phase::Ended,
                    }
                };
                let (horizontal, direction) = this.gesture.update(
                    delta.x.into(),
                    delta.y.into(),
                    phase,
                    std::time::Instant::now(),
                );
                if let Some(back) = direction {
                    this.command(if back { "back" } else { "forward" }, window, cx);
                }
                if horizontal {
                    cx.stop_propagation();
                }
            }))
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
                                    .h(px(if this.settings.compact { 28. } else { 32. }
                                        * this.settings.ui_scale))
                                    .px_3()
                                    .py_1()
                                    .bg(if selected {
                                        cx.theme().selection
                                    } else {
                                        cx.theme().sidebar
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
                                            this.open(path.clone(), event.click_count() < 2, None);
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
            .id("tabs")
            .w_full()
            .flex()
            .gap_1()
            .p(px(4.))
            .overflow_x_scroll()
            .flex_shrink_0()
            .track_scroll(&self.tab_scroll)
            .children(self.documents.iter().map(|doc| {
                let id = doc.id;
                let title = format!(
                    "{}{}{}{}",
                    paths::tab_label(
                        &doc.disk.path,
                        self.documents.iter().map(|doc| doc.disk.path.as_path())
                    ),
                    if doc.dirty(cx) { " •" } else { "" },
                    if doc.preview { " (preview)" } else { "" },
                    if doc.external.is_some() {
                        " (conflict)"
                    } else {
                        ""
                    }
                );
                div().track_focus(&doc.tab_focus).flex_shrink_0().child(
                    Button::new(("tab", id))
                        .label(title)
                        .tooltip(paths::display(&doc.disk.path))
                        .small()
                        .compact()
                        .selected(Some(id) == self.active)
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            this.active = Some(id);
                            if event.click_count() > 1
                                && let Some(doc) =
                                    this.documents.iter_mut().find(|doc| doc.id == id)
                            {
                                doc.preview = false;
                            }
                            this.reading = false;
                            this.navigation_only = false;
                            this.focus(window, cx);
                            cx.notify();
                        })),
                )
            }));
        let mut content = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs);
        if !self.request.pick {
            let actions = div().flex().flex_wrap().gap_2().p_1();
            content = content.child(if narrow {
                actions
                    .child(self.button("Save", "save", cx))
                    .child(self.button("Read / Edit", "read", cx))
                    .child(self.button("Close", "close", cx))
            } else {
                actions
                    .child(self.button("New", "new", cx))
                    .child(self.button("Save", "save", cx))
                    .child(self.button("Save As", "save-as", cx))
                    .child(self.button("Save Copy", "save-copy", cx))
                    .child(self.button("Read / Edit", "read", cx))
                    .child(self.button("Line", "line", cx))
                    .child(self.button("Close tab", "close", cx))
                    .child(self.button("Reload disk", "reload", cx))
            });
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            if doc.recovery_error.is_none() && doc.error.is_some() {
                content = content.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .p_2()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(match doc.failure_action {
                            FailureAction::Save => "Last save failed",
                            FailureAction::Reload => "Reload stopped",
                            FailureAction::Check => "File unavailable",
                            FailureAction::Choose(_) => "Destination unavailable",
                        })
                        .child(self.button("Details and Retry…", "failure-details", cx)),
                );
            }
            if let Some((_, error)) = &doc.recovery_error {
                content = content.child(self.recovery_failure_view(doc, error, cx));
            }
            if doc.external.is_some() {
                content = content.child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .p_2()
                        .child("Changed on disk. Local text retained; ordinary Save is blocked.")
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(self.button("Compare", "compare", cx))
                                .child(self.button("Keep Editing", "keep-editing", cx))
                                .child(self.command_button(
                                    "conflict-reload",
                                    "Reload Disk Version",
                                    "reload",
                                    cx,
                                ))
                                .child(self.command_button(
                                    "conflict-save-copy",
                                    "Save Copy",
                                    "save-copy",
                                    cx,
                                ))
                                .child(self.button("Overwrite Disk Version…", "overwrite", cx)),
                        ),
                );
            }
            if self.reading {
                content = content.child(
                    div()
                        .id("reading")
                        .flex_1()
                        .overflow_y_scroll()
                        .p_4()
                        .text_size(px(self.settings.markdown_font_size * self.settings.ui_scale))
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
                        .font_family(
                            if self.settings.proportional
                                && glimpse::syntax::grammar_extension(&doc.disk.path, &doc.saved)
                                    .is_none()
                            {
                                cx.theme().font_family.clone()
                            } else {
                                cx.theme().mono_font_family.clone()
                            },
                        )
                        .child(
                            Input::new(&doc.input)
                                .text_size(px(self.settings.font_size * self.settings.ui_scale))
                                .line_height(relative(self.settings.line_height))
                                .h_full()
                                .disabled(self.request.pick),
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
        let body = if narrow {
            if self.navigation_only || self.active.is_none() {
                navigator.into_any_element()
            } else {
                content.into_any_element()
            }
        } else if self.navigator_visible {
            div()
                .flex()
                .size_full()
                .child(
                    div()
                        .w(self.pane_state.read(cx).width(window))
                        .h_full()
                        .flex_shrink_0()
                        .child(navigator),
                )
                .child(self.pane_state.clone())
                .child(content)
                .into_any_element()
        } else {
            content.into_any_element()
        };
        root.child(div().flex_1().min_h_0().child(body))
            .when(!self.warnings.is_empty(), |root| {
                root.child(
                    div()
                        .p_2()
                        .child(self.button("Recovery Warning…", "warnings", cx)),
                )
            })
            .when(
                !self.status.is_empty() && self.warnings.is_empty(),
                |root| {
                    root.child(
                        div()
                            .p_2()
                            .text_size(px(12. * self.settings.ui_scale))
                            .child(self.status.clone()),
                    )
                },
            )
            .children(self.preferences.clone())
            .children(self.decision.clone())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.preferences.is_some() || this.decision.is_some() {
                    return;
                }
                let key = &event.keystroke;
                if this.nav_focus.is_focused(window)
                    && !key.modifiers.control
                    && !key.modifiers.platform
                {
                    let query = this.filter.to_lowercase();
                    let rows: Vec<_> = this
                        .entries
                        .iter()
                        .filter(|entry| {
                            (this.settings.show_hidden
                                || !paths::name(&entry.path).starts_with('.'))
                                && paths::name(&entry.path).to_lowercase().contains(&query)
                        })
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
                            if let Some(path) = this.selected_path() {
                                this.open(path, false, None);
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
                    this.command("dismiss", window, cx);
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
