mod accessibility;
mod completion;
mod conflict;
mod decision;
mod destination;
mod external;
mod menus;
mod modal;
mod open_failure;
mod panes;
mod path;
mod platform;
mod preferences;
mod reading;
mod recovery;
mod searching;
mod theme;

use glimpse::directory::Entry;
use glimpse::{
    cli, directory,
    document::{DiskFile, Observation},
    io::{IoWorker, Job, Reply, SaveDestination},
    lifecycle, observe, paths, search, storage, wake,
};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Root, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Position, PreparedInput, Rope},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

pub fn run(request: cli::Request, data_dir: PathBuf, launch_folder: PathBuf) {
    platform::configure_allocations();
    let startup = StartupIo::new(&request, &data_dir);
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
                    window.set_window_title("Glimpse");
                    let view = cx.new(|cx| Shell::new(request, startup, launch_folder, window, cx));
                    let weak = view.downgrade();
                    window.on_window_should_close(cx, move |window, cx| {
                        weak.update(cx, |shell, cx| {
                            shell.close(true, None, window, cx);
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
    disk_comparison: RefCell<Option<(Rope, bool)>>,
    status_cursor: usize,
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
    external: Option<(DiskFile, std::sync::Arc<str>)>,
    deferred_external: Option<(DiskFile, std::sync::Arc<str>)>,
    passive_refresh: bool,
    tab_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}
impl Document {
    fn watched_paths(&self) -> impl Iterator<Item = &PathBuf> {
        [
            Some(&self.disk),
            self.external.as_ref().map(|(disk, _)| disk),
            self.deferred_external.as_ref().map(|(disk, _)| disk),
        ]
        .into_iter()
        .flatten()
        .filter(|disk| !disk.target.as_os_str().is_empty())
        .flat_map(|disk| [&disk.path, &disk.target])
    }
    fn information(&self, cx: &App, reading: bool) -> String {
        let input = self.input.read(cx);
        let endings = input.line_endings();
        let encoding = if self.disk.bom { "UTF-8 BOM" } else { "UTF-8" };
        if reading {
            return format!("Read · {encoding} · {endings}");
        }
        let position = input.cursor_position();
        format!(
            "Ln {}, Col {} · {encoding} · {endings}",
            position.line + 1,
            position.character + 1,
        )
    }
    fn dirty(&self, cx: &App) -> bool {
        if self.restored {
            return true;
        }
        let input = self.input.read(cx);
        let mut comparison = self.disk_comparison.borrow_mut();
        if let Some((revision, clean)) = comparison.as_ref()
            && input.has_text_revision(revision)
        {
            return !clean;
        }
        let text = input.text();
        let clean = self
            .disk
            .text_revision()
            .is_some_and(|revision| input.has_text_revision(revision))
            || self.disk.matches_revision(text);
        *comparison = Some((text.clone(), clean));
        !clean
    }

    fn set_disk(&mut self, disk: DiskFile) {
        self.disk = disk;
        self.disk_comparison.get_mut().take();
    }

    fn grammar(&self, cx: &App) -> Option<&'static str> {
        if self.restored {
            return glimpse::syntax::grammar_for_text(&self.disk.path, self.input.read(cx).text());
        }
        let content = self.disk.content_prefix(256);
        let first_line = if content.starts_with(b"#!") {
            String::from_utf8_lossy(&content)
        } else {
            "".into()
        };
        glimpse::syntax::grammar_extension(&self.disk.path, &first_line)
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
    FailureDetails(Box<destination::Failure>),
    Warnings,
    Notice,
    FolderError,
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
    Locations {
        visits: Vec<PathBuf>,
        last: Option<PathBuf>,
    },
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
    hits: std::sync::Arc<Vec<search::Hit>>,
    search_ui: searching::State,
    scope: Option<PathBuf>,
    current: PathBuf,
    launch_folder: PathBuf,
    history: Vec<PathBuf>,
    gesture: glimpse::gesture::Gesture,
    future: Vec<PathBuf>,
    nav_focus: FocusHandle,
    reading_focus: FocusHandle,
    pane_state: Entity<panes::Divider>,
    navigation_only: bool,
    navigator_visible: bool,
    scrolls: HashMap<PathBuf, UniformListScrollHandle>,
    selection: Option<PathBuf>,
    reveal_pending: Option<PathBuf>,
    selections: HashMap<PathBuf, PathBuf>,
    entries: std::sync::Arc<Vec<Entry>>,
    entries_revision: u64,
    visible_key: Option<(u64, String, bool)>,
    visible_entries: std::sync::Arc<Vec<usize>>,
    listing_buffer: Vec<Entry>,
    listing_streaming: bool,
    listing_error: Option<String>,
    listing_waiter: Option<Task<()>>,
    listing_stalled: bool,
    listing_paused: bool,
    listing_id: u64,
    listing_done: bool,
    restoring_location: bool,
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
    hit_versions: HashMap<u64, (u64, u64)>,
    panel: Option<Panel>,
    query: Entity<InputState>,
    reading: bool,
    reading_view: Option<reading::View>,
    reading_request: u64,
    memory_release: Option<Task<()>>,
    status: String,
    open_failure: Option<open_failure::Failure>,
    font_warning: Option<String>,
    window_title: String,
    warnings: Vec<String>,
    omitted_warnings: u64,
    volumes: Option<glimpse::locations::Volumes>,
    settings: storage::Settings,
    settings_write: Option<SettingsWrite>,
    settings_loaded: bool,
    settings_reloading: bool,
    location_visits_pending: Vec<PathBuf>,
    last_location_pending: Option<PathBuf>,
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
    startup_recovery: Option<recovery::Startup>,
    ready: bool,
    startup_error: Option<String>,
    requested: Vec<(u64, PathBuf)>,
    finished: bool,
    validation_disks: HashMap<u64, DiskFile>,
    cleanup_texts: HashMap<u64, (u64, String)>,
    completed: HashSet<u64>,
    validated: HashSet<u64>,
    keep_after_cleanup: HashSet<u64>,
    finishing: glimpse::validation::Validations,
}
struct StartupIo {
    wake: wake::Context,
    replies_ready: async_channel::Receiver<()>,
    io: IoWorker,
    opening: HashMap<u64, OpenIntent>,
}

impl StartupIo {
    fn new(request: &cli::Request, data_dir: &Path) -> Self {
        let (tx, replies_ready) = async_channel::bounded(1);
        let wake = wake::Context::new(move || {
            let _ = tx.try_send(());
        });
        let io = IoWorker::new(wake.clone(), data_dir);
        io.send(Job::LoadSettings);
        io.send(Job::Volumes);
        if request.desktop {
            io.send(Job::Inventory);
        }
        let mut opening = HashMap::with_capacity(request.files.len());
        for (index, path) in request.files.iter().enumerate() {
            let token = index as u64 + 1;
            let position = (index == 0).then_some((request.line, request.column));
            opening.insert(
                token,
                OpenIntent {
                    preview: false,
                    position,
                    interaction: 0,
                },
            );
            io.send(Job::Open(path.clone(), token, false));
        }
        Self {
            wake,
            replies_ready,
            io,
            opening,
        }
    }
}

impl Shell {
    fn new(
        request: cli::Request,
        startup: StartupIo,
        launch_folder: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let StartupIo {
            wake,
            replies_ready,
            io,
            opening,
        } = startup;
        let open_token = request.files.len() as u64;
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
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| launch_folder.clone());
        let query = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Enter a path");
            input.set_external_highlights(Vec::new(), cx);
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
                this.search_changed(cx);
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
            while replies_ready.recv().await.is_ok() {
                if this
                    .update_in(cx, |this, window, cx| this.receive(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let startup_recovery = request.desktop.then(recovery::Startup::new);
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
            launch_folder,
            history: vec![],
            gesture: glimpse::gesture::Gesture::default(),
            future: vec![],
            nav_focus: cx.focus_handle().tab_stop(true),
            reading_focus: cx.focus_handle().tab_stop(true),
            pane_state: cx.new(panes::Divider::new),
            navigation_only: false,
            navigator_visible: true,
            scrolls: HashMap::new(),
            selection: None,
            reveal_pending: None,
            selections: HashMap::new(),
            entries: Default::default(),
            entries_revision: 0,
            visible_key: None,
            visible_entries: Default::default(),
            listing_buffer: vec![],
            listing_streaming: true,
            listing_error: None,
            listing_waiter: None,
            listing_stalled: false,
            listing_paused: false,
            listing_id: 0,
            listing_done: false,
            restoring_location: false,
            filter: String::new(),
            documents: vec![],
            active: None,
            tab_scroll: ScrollHandle::new(),
            next_id: 1,
            open_token,
            interaction: 0,
            path_request: None,
            path_completion_token: 0,
            path_completions: None,
            opening,
            hit_versions: HashMap::new(),
            panel: None,
            query,
            reading: false,
            reading_view: None,
            reading_request: 0,
            memory_release: None,
            status: String::new(),
            open_failure: None,
            font_warning: None,
            window_title: String::new(),
            warnings: Vec::new(),
            omitted_warnings: 0,
            volumes: None,
            settings: storage::Settings::default(),
            settings_write: None,
            settings_loaded: false,
            settings_reloading: true,
            location_visits_pending: Vec::new(),
            last_location_pending: None,
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
            startup_recovery,
            ready: false,
            startup_error: None,
            requested: vec![],
            finished: false,
            validation_disks: HashMap::new(),
            cleanup_texts: HashMap::new(),
            completed: HashSet::new(),
            validated: HashSet::new(),
            keep_after_cleanup: HashSet::new(),
            finishing: glimpse::validation::Validations::default(),
            search_id: 0,
            hits: Default::default(),
            search_ui: searching::State::new(cx),
        };
        window.focus(&this.nav_focus);
        this.refresh();
        this.subscriptions
            .push(cx.observe(&this.pane_state, |_, _, cx| cx.notify()));
        this.subscriptions
            .push(cx.observe_window_activation(window, |this, window, cx| {
                if this.reading {
                    this.prepare_inactive_reading(window, cx);
                }
            }));
        this
    }
    fn report_startup_ready(&mut self, source_visible: bool, window: &mut Window, cx: &App) {
        if !self.ready
            && (!self.request.desktop || self.settings_loaded)
            && self.listing_done
            && self.opening.is_empty()
            && self.startup_recovery.is_none()
            && (!source_visible
                || !self.documents.iter().any(|doc| {
                    Some(doc.id) == self.active && doc.input.read(cx).initial_layout_pending()
                }))
        {
            self.ready = true;
            let error = self.startup_error.take().or_else(|| {
                self.listing_error
                    .as_ref()
                    .map(|error| format!("{}: {error}", paths::display(&self.current)))
            });
            let timing = std::env::var_os("GLIMPSE_TIMING").is_some();
            window.on_next_frame(move |window, _| {
                if timing && error.is_none() {
                    eprintln!("GLIMPSE_EDITABLE_FRAME");
                }
                window.on_next_frame(move |_, _| {
                    if let Some(error) = error {
                        let _ = lifecycle::notify_startup_failure(&error);
                        eprintln!("GLIMPSE_STARTUP_FAILED");
                    } else {
                        let _ = lifecycle::notify(b'0');
                        eprintln!("GLIMPSE_READY");
                    }
                })
            });
        }
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
                    settings.last_location = this.settings.last_location.clone();
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
            self.last_location_pending = None;
            return;
        }
        let visits = std::mem::take(&mut self.location_visits_pending);
        let mut settings = self.settings.clone();
        let last = self
            .last_location_pending
            .take()
            .filter(|path| settings.last_location.as_ref() != Some(path));
        if let Some(path) = &last {
            settings.last_location = Some(path.clone());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut changed = last.is_some();
        for path in &visits {
            changed |= settings.record_location(path, now);
        }
        if changed {
            let _ = self.queue_settings(settings, SettingsWrite::Locations { visits, last });
        }
    }

    fn apply_settings(
        &mut self,
        settings: storage::Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if settings.remember_locations
            && !self.settings.remember_locations
            && self.listing_done
            && self.listing_error.is_none()
            && !self.request.pick
        {
            self.last_location_pending = Some(self.current.clone());
        }
        self.settings = settings;
        if let Some(warning) = &self.settings.warning {
            self.status = warning.clone();
        }
        self.font_warning = theme::apply(&self.settings, window, cx);
        self.reset_search("Search settings changed; press Enter to search");
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
        let closing = matches!(&action, DecisionAction::Close);
        let view = cx.new(|cx| {
            let mut decision = decision::Decision::new(title, detail, actions, window, cx);
            if closing {
                decision.close_actions();
            }
            decision
        });
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
                        Some(DecisionAction::FailureDetails(failure))
                            if this.documents.iter().any(|doc| doc.id == failure.id) =>
                        {
                            this.active = Some(failure.id);
                            match choice.0 {
                                1 => this.retry_failure(*failure, window, cx),
                                2 => this.save_as(DestinationKind::Copy, window, cx),
                                _ => this.focus(window, cx),
                            }
                        }
                        Some(DecisionAction::FolderError) if choice.0 == 1 => this.refresh(),
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
        let paths = self
            .documents
            .iter()
            .flat_map(Document::watched_paths)
            .cloned()
            .collect();
        self.observer.update(self.current.clone(), paths);
    }
    fn warn(&mut self, error: String) {
        if self.warnings.contains(&error) {
            return;
        }
        if self.warnings.len() == 32 {
            self.warnings.remove(0);
            self.omitted_warnings = self.omitted_warnings.saturating_add(1);
        }
        self.warnings.push(error);
    }
    fn check_document(&mut self, id: u64) {
        if self
            .documents
            .iter()
            .any(|d| d.id == id && !d.saving && !d.disk.target.as_os_str().is_empty())
        {
            self.checks.request(id);
            self.dispatch_checks();
        }
    }
    fn dispatch_checks(&mut self) {
        loop {
            let documents = &self.documents;
            let Some((id, token)) = self.checks.next(|id| {
                documents
                    .iter()
                    .find(|doc| {
                        doc.id == id && !doc.saving && !doc.disk.target.as_os_str().is_empty()
                    })
                    .map(|doc| doc.generation)
            }) else {
                break;
            };
            let doc = self.documents.iter().find(|doc| doc.id == id).unwrap();
            glimpse::metrics::event("check-request", id, token);
            self.io.send(Job::Check(id, doc.disk.clone(), token));
        }
    }
    fn visible_rows(&mut self) -> std::sync::Arc<Vec<usize>> {
        let key = (
            self.entries_revision,
            self.filter.to_lowercase(),
            self.settings.show_hidden,
        );
        if self.visible_key.as_ref() != Some(&key) {
            self.visible_entries = std::sync::Arc::new(
                self.entries
                    .iter()
                    .enumerate()
                    .filter_map(|(i, entry)| {
                        let name = paths::name(&entry.path);
                        ((self.settings.show_hidden
                            || !entry
                                .path
                                .file_name()
                                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".")))
                            && name.to_lowercase().contains(&key.1))
                        .then_some(i)
                    })
                    .collect(),
            );
            self.visible_key = Some(key);
        }
        self.visible_entries.clone()
    }
    fn refresh(&mut self) {
        self.listing_buffer.clear();
        self.listing_streaming =
            self.entries.is_empty() && !self.scrolls.contains_key(&self.current);
        self.listing_done = false;
        self.listing_error = None;
        self.listing_waiter = None;
        self.listing_stalled = false;
        self.listing_paused = false;
        self.directories.retain(std::slice::from_ref(&self.current));
        match self.directories.request(self.current.clone()) {
            Ok(id) => {
                self.listing_id = id;
                glimpse::metrics::event("directory-request", id, 0);
            }
            Err(error) => {
                self.listing_id = 0;
                self.listing_done = true;
                if !self.ready && (!self.request.desktop || self.settings_loaded) {
                    self.startup_error.get_or_insert_with(|| {
                        format!("{}: {error}", paths::display(&self.current))
                    });
                }
                self.listing_error = Some(error);
            }
        }
        self.watch();
    }
    fn navigate(&mut self, path: PathBuf) {
        if path != self.current {
            self.history.push(self.current.clone());
            self.future.clear();
            self.change_location(path);
        }
    }
    fn set_initial_location(&mut self, path: PathBuf) {
        self.current = path;
        self.entries = Default::default();
        self.entries_revision += 1;
        self.selection = None;
        self.last_location_pending = None;
        self.refresh();
    }
    fn change_location(&mut self, path: PathBuf) {
        self.restoring_location = false;
        let old_scope = self.effective_scope().clone();
        if let Some(selection) = self.selection.take() {
            self.selections.insert(self.current.clone(), selection);
        }
        self.interaction += 1;
        self.reveal_pending = None;
        self.path_completions = None;
        self.path_completion_token += 1;
        self.io.cancel_previews(self.open_token + 1);
        self.entries = Default::default();
        self.entries_revision += 1;
        self.search_ui.picker_selection = None;
        self.current = path;
        self.selection = self.selections.get(&self.current).cloned();
        self.filter.clear();
        if self.effective_scope() != &old_scope {
            self.reset_search("Search folder changed; press Enter to search");
        }
        self.refresh();
    }
    fn selected_path(&self) -> Option<PathBuf> {
        self.selection
            .as_ref()
            .filter(|path| {
                path.parent() == Some(self.current.as_path())
                    && self.entries.iter().any(|entry| entry.path == **path)
                    && (self.settings.show_hidden
                        || !path
                            .file_name()
                            .is_some_and(|name| name.as_encoded_bytes().starts_with(b".")))
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
        text: PreparedInput,
        intent: OpenIntent,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let OpenIntent {
            preview, position, ..
        } = intent;
        if activate && self.request.pick {
            self.search_ui.picker_selection = Some(disk.path.clone());
        }
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
            let release = self.documents.iter().any(|doc| {
                doc.preview && !doc.dirty(cx) && doc.input.read(cx).text().len() >= 1024 * 1024
            });
            self.documents.retain(|d| !d.preview || d.dirty(cx));
            if release {
                self.release_view_memory(window, cx);
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .code_editor("text")
                .line_number(self.settings.line_numbers)
                .soft_wrap(self.settings.wrap)
                .defer_large_wrapped_layout()
                .default_prepared_value(text);
            if disk.text_revision().is_some() {
                input.release_value_cache(cx);
            }
            input.set_external_highlights(Vec::new(), cx);
            input.set_cursor_blink(false, cx);
            input.set_accessibility_label(format!("Source: {}", paths::display(&disk.path)), cx);
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
        let subscription = cx.subscribe_in(&input, window, move |this, _, event, window, cx| {
            if matches!(event, InputEvent::InitialLayoutReady) {
                let source_visible =
                    !this.reading && (!this.narrow(window) || !this.navigation_only);
                this.report_startup_ready(source_visible, window, cx);
                cx.notify();
                return;
            }
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
                if !doc.passive_refresh || doc.dirty(cx) {
                    doc.preview = false;
                }
                doc.passive_refresh = false;
                if !this.request.pick && !doc.dirty(cx) {
                    this.io.send(Job::ClearThrough(id, doc.generation));
                } else if !this.request.pick {
                    this.io.recover_edit(
                        id,
                        storage::Draft {
                            path: doc.disk.path.clone(),
                            target: Some(doc.disk.target.clone()),
                            text: doc.input.read(cx).text().clone(),
                            base: doc.disk.bytes.clone(),
                            generation: doc.generation,
                        },
                    );
                }
            }
            if let Some(doc) = this.documents.iter().find(|d| d.id == id) {
                this.syntax.request(
                    id,
                    doc.generation,
                    doc.input.read(cx).text().clone(),
                    &doc.disk.path,
                    cx.theme().is_dark(),
                );
            }
            cx.notify();
        });
        if self.request.files.contains(&disk.path) {
            self.requested.push((id, disk.path.clone()));
        }
        let refresh_subscription = cx.observe_in(&input, window, move |this, _, window, cx| {
            if this.active == Some(id)
                && let Some(doc) = this.documents.iter_mut().find(|doc| doc.id == id)
            {
                let cursor = doc.input.read(cx).cursor();
                if doc.status_cursor != cursor {
                    doc.status_cursor = cursor;
                    cx.notify();
                }
            }
            this.apply_external(id, window, cx);
        });
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
            disk_comparison: RefCell::default(),
            status_cursor: 0,
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
            deferred_external: None,
            passive_refresh: false,
            tab_focus,
            _subscriptions: vec![subscription, tab_subscription, refresh_subscription],
        });
        self.tab_scroll.scroll_to_item(self.documents.len() - 1);
        self.syntax.request(
            id,
            0,
            input.read(cx).text().clone(),
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
        let clamped = input.update(cx, |input, cx| {
            let _work = gpui::work_trace::Span::new("editor_position");
            let (offset, clamped) = glimpse::editor::source_position(input.text(), line, column);
            let cursor = input.set_cursor_offset_unfocused(offset, cx);
            if focus {
                input.focus(window, cx);
            }
            clamped || cursor != offset
        });
        if clamped {
            self.status = "Requested position clamped to a text boundary".into();
        }
    }

    fn release_view_memory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (released, rendered) = async_channel::bounded(1);
        window.on_next_frame(move |window, _| {
            window.refresh();
            window.on_next_frame(move |_, _| {
                let _ = released.try_send(());
            });
        });
        self.memory_release = Some(cx.background_spawn(async move {
            if rendered.recv().await.is_ok() {
                Timer::after(std::time::Duration::from_millis(200)).await;
                let _work = gpui::work_trace::Span::new("view_memory_release");
                platform::release_unused_memory();
            }
        }));
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
                doc.input.read(cx).text().clone(),
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
        if panel == Panel::Palette && matches!(self.panel, Some(Panel::Files | Panel::Text)) {
            self.search_ui.suspended = self
                .panel
                .map(|panel| (panel, self.query.read(cx).value().to_string()));
        } else {
            self.search_ui.suspended = None;
            if !matches!(panel, Panel::Files | Panel::Text) {
                self.search_ui.temporary = None;
            }
            self.reset_search("Enter a query, then press Enter to search");
            self.search_ui.query.clear();
        }
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
            if panel == Panel::Path {
                input.select_all(&gpui_component::input::SelectAll, window, cx);
            }
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
                    self.palette_command(command.id, window, cx);
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
            Some(Panel::Files | Panel::Text) => self.start_search(cx),
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
        let mut listing_changed = false;
        while let Ok(batch) = self.directories.replies.try_recv() {
            if batch.path == self.current && batch.generation == self.listing_id {
                if self.listing_buffer.is_empty() && !batch.entries.is_empty() {
                    glimpse::metrics::event(
                        "directory-first",
                        batch.generation,
                        batch.entries.len() as u64,
                    );
                }
                self.listing_buffer.extend(batch.entries);
                listing_changed = true;
                self.listing_done = batch.done;
                if batch.done {
                    glimpse::metrics::event(
                        "directory-complete",
                        batch.generation,
                        self.listing_buffer.len() as u64,
                    );
                }
                if let Some(error) = batch.error {
                    if self.restoring_location {
                        self.restoring_location = false;
                        if let Some(home) = std::env::home_dir() {
                            self.set_initial_location(home);
                            self.status = "The last folder is unavailable. Showing Home.".into();
                        } else {
                            self.set_initial_location(self.launch_folder.clone());
                            self.status =
                                "The last folder is unavailable. Showing the launch folder.".into();
                        }
                        listing_changed = false;
                        continue;
                    }
                    if !self.ready && (!self.request.desktop || self.settings_loaded) {
                        self.startup_error.get_or_insert_with(|| {
                            format!("{}: {error}", paths::display(&self.current))
                        });
                    }
                    self.listing_error = Some(error);
                } else if batch.done {
                    self.restoring_location = false;
                    if !self.request.pick && self.listing_error.is_none() {
                        self.last_location_pending = Some(self.current.clone());
                    }
                }
            }
        }
        if listing_changed && (self.listing_streaming || self.listing_done) {
            self.listing_buffer.sort_by(|a, b| {
                b.directory
                    .cmp(&a.directory)
                    .then_with(|| a.path.cmp(&b.path))
            });
            if self.listing_error.is_none()
                || !self.listing_buffer.is_empty()
                || self.entries.is_empty()
            {
                self.entries = std::sync::Arc::new(if self.listing_done {
                    std::mem::take(&mut self.listing_buffer)
                } else {
                    self.listing_buffer.clone()
                });
                self.entries_revision += 1;
                glimpse::metrics::event(
                    "directory-installed",
                    self.listing_id,
                    self.entries.len() as u64,
                );
            }
        }
        while let Ok(reply) = self.io.replies.try_recv() {
            match reply {
                Reply::Markdown(request, result) => self.reading_ready(request, result, cx),
                Reply::PathCompleted(token, value, result) => {
                    self.path_completed(token, value, result, window, cx)
                }
                Reply::Volumes(result) => match result {
                    Ok(volumes) => self.volumes = Some(volumes),
                    Err(error) => self.status = format!("Mounted locations: {error}"),
                },
                Reply::OpenDirectory(path, token) => {
                    if let Some(intent) = self.opening.remove(&token) {
                        if !self.ready && self.request.files.contains(&path) {
                            self.status = format!(
                                "Requested file became a directory: {}",
                                paths::display(&path)
                            );
                            self.startup_error
                                .get_or_insert_with(|| self.status.clone());
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
                    let expected = self.hit_versions.remove(&token);
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
                                if self
                                    .open_failure
                                    .as_ref()
                                    .is_some_and(|failure| failure.path == path)
                                {
                                    self.open_failure = None;
                                }
                                if self.destination_busy {
                                    continue;
                                }
                                if let Some((expected, search_generation)) = expected
                                    && (expected
                                        != search::fingerprint_body(
                                            text.value().as_bytes(),
                                            disk.bom,
                                        )
                                        || self
                                            .documents
                                            .iter()
                                            .any(|doc| doc.disk.same_file(&disk) && doc.dirty(cx)))
                                {
                                    let message = "Failed to open result — disk changed or the document has unsaved edits. Search again before jumping.";
                                    self.status = message.into();
                                    if search_generation == self.search_id {
                                        self.search_ui.status = message.into();
                                    }
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
                                if !self.ready && self.request.files.contains(&path) {
                                    self.startup_error.get_or_insert_with(|| {
                                        format!("{}: {e}", paths::display(&path))
                                    });
                                }
                                if expected
                                    .is_some_and(|(_, generation)| generation == self.search_id)
                                {
                                    self.search_ui.status = format!("Failed to open result: {e}");
                                }
                                self.open_failure = Some(open_failure::Failure {
                                    path,
                                    message: e.clone(),
                                });
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
                                text: doc.input.read(cx).value().into(),
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
                                let release = self.documents.iter().any(|doc| {
                                    doc.id == id && doc.input.read(cx).text().len() >= 1024 * 1024
                                });
                                self.documents.retain(|d| d.id != id);
                                if release {
                                    self.release_view_memory(window, cx);
                                }
                                self.checks.invalidate(id);
                                self.watch();
                                if self.active == Some(id) {
                                    self.active = self.documents.last().map(|d| d.id);
                                    self.reading = false;
                                }
                            }
                            self.complete_request();
                            if self.reading {
                                window.focus(&self.reading_focus);
                            } else {
                                self.focus(window, cx);
                            }
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
                Reply::Save(id, generation, result) => {
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
                                doc.set_disk(disk);
                                if doc.disk.text_revision().is_some() {
                                    doc.input
                                        .update(cx, |input, cx| input.release_value_cache(cx));
                                }
                                doc.restored = false;
                                doc.external = None;
                                doc.generation += 1;
                                if doc.dirty(cx) {
                                    self.io.recover_edit(
                                        id,
                                        storage::Draft {
                                            path: doc.disk.path.clone(),
                                            target: Some(doc.disk.target.clone()),
                                            text: doc.input.read(cx).text().clone(),
                                            base: doc.disk.bytes.clone(),
                                            generation: doc.generation,
                                        },
                                    );
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
                                doc.deferred_external = None;
                                doc.set_disk(disk);
                                doc.input.update(cx, |input, cx| {
                                    input.set_accessibility_label(
                                        format!("Source: {}", paths::display(&doc.disk.path)),
                                        cx,
                                    )
                                });
                                doc.external = None;
                                doc.restored = false;
                                doc.generation += 1;
                                if doc.dirty(cx) {
                                    self.io.send(Job::Recover(
                                        doc.id,
                                        storage::Draft {
                                            path: doc.disk.path.clone(),
                                            target: Some(doc.disk.target.clone()),
                                            text: doc.input.read(cx).value().into(),
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
                    self.watch();
                    self.check_document(id);
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
                        .find(|d| d.id == id && !d.saving)
                        .map(|d| d.generation);
                    if self.checks.complete(id, token, generation)
                        && let Some(doc) =
                            self.documents.iter_mut().find(|d| d.id == id && !d.saving)
                    {
                        if result.is_ok() && doc.failure_action == FailureAction::Check {
                            doc.error = None;
                            doc.failure_action = FailureAction::Save;
                            self.status = "File access restored".into();
                        }
                        match result {
                            Ok(Observation::Changed(disk, text))
                                if disk.target != doc.disk.target =>
                            {
                                doc.deferred_external = None;
                                self.status = format!(
                                    "Destination changed from {} to {}. Local text retained; use Save Copy or explicitly Reload disk.",
                                    paths::display(&doc.disk.target),
                                    paths::display(&disk.target)
                                );
                                doc.external = Some((disk, text));
                            }
                            Ok(Observation::Changed(disk, text))
                                if disk.bytes != doc.disk.bytes =>
                            {
                                doc.deferred_external = Some((disk, text));
                            }
                            Ok(
                                Observation::Unchanged(mut disk)
                                | Observation::Changed(mut disk, _),
                            ) => {
                                if doc.disk.bom == disk.bom {
                                    disk.bytes = doc.disk.bytes.clone();
                                }
                                doc.deferred_external = None;
                                doc.external = None;
                                doc.set_disk(disk);
                            }
                            Err(error) => {
                                doc.error = Some(error.clone());
                                doc.failure_action = FailureAction::Check;
                                doc.failed_destination = None;
                                self.status = format!("File unavailable: {error}");
                            }
                        }
                        self.apply_external(id, window, cx);
                        self.watch();
                    }
                    self.dispatch_checks();
                }
                Reply::CheckDeferred(id, token) => {
                    glimpse::metrics::event("check-deferred", id, token);
                    cx.spawn(async move |this, cx| {
                        Timer::after(std::time::Duration::from_millis(100)).await;
                        let _ = this.update(cx, |this, _| {
                            this.checks.retry(id, token);
                            this.dispatch_checks();
                        });
                    })
                    .detach();
                }
                Reply::Reloaded(reload, result) => self.reloaded(*reload, result, window, cx),
                Reply::SettingsLoaded(result) => {
                    self.settings_reloading = false;
                    let restore_location =
                        self.request.desktop && !self.settings_loaded && self.interaction == 0;
                    self.settings_loaded = true;
                    match result {
                        Ok(settings) => {
                            if restore_location
                                && settings.remember_locations
                                && let Some(path) = settings.last_location.as_ref()
                                && path.is_absolute()
                                && path != &self.current
                            {
                                self.restoring_location = true;
                                self.set_initial_location(path.clone());
                            }
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
                                SettingsWrite::Locations { .. } => self.settings = settings,
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
                                SettingsWrite::Locations { visits, last } => {
                                    if self.last_location_pending.is_none() {
                                        self.last_location_pending = last;
                                    }
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
                        if let Some(startup) = &mut self.startup_recovery
                            && startup.inventory_pending
                        {
                            startup.inventory_pending = false;
                            startup.paths = self
                                .records
                                .iter()
                                .filter(|record| !record.history && !record.candidate)
                                .map(|record| record.path.clone())
                                .collect();
                            if self.records_truncated || self.recovery_usage.partial {
                                self.warnings.push("Recovery inventory is incomplete. Open Recovery to inspect remaining records after restoration.".into());
                            }
                            self.restore_next_startup();
                        }
                    }
                    Err(error) => {
                        if self
                            .startup_recovery
                            .as_ref()
                            .is_some_and(|startup| startup.inventory_pending)
                        {
                            self.startup_recovery = None;
                            self.startup_error.get_or_insert_with(|| {
                                format!("Startup recovery could not be read: {error}")
                            });
                            self.warnings.push(format!("Startup recovery could not be read: {error}. Open Recovery to retry."));
                        }
                        self.status = error;
                    }
                },
                Reply::Imported(import_id, result) => {
                    let automatic = self
                        .startup_recovery
                        .as_ref()
                        .is_some_and(|startup| startup.current == Some(import_id));
                    let activate = !automatic || (self.interaction == 0 && self.active.is_none());
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
                                PreparedInput::new(draft.text),
                                OpenIntent {
                                    preview: false,
                                    position: None,
                                    interaction: self.interaction,
                                },
                                activate,
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
                                        text: doc.input.read(cx).value().into(),
                                        base: doc.disk.bytes.clone(),
                                        generation: doc.generation,
                                    },
                                ));
                            }
                            if !automatic {
                                self.panel = None;
                            }
                            if automatic && let Some(startup) = &mut self.startup_recovery {
                                startup.restored += 1;
                            }
                            self.status = if duplicate {
                                "Restored a separate copy because the original is already open. Use Save As.".into()
                            } else {
                                "Recovered local text. The original file has not been changed."
                                    .into()
                            };
                            self.check_document(import_id);
                        }
                        Err(error) => {
                            self.status = error;
                            if automatic && let Some(startup) = &mut self.startup_recovery {
                                startup.failed += 1;
                                self.startup_error.get_or_insert_with(|| {
                                    format!(
                                        "Startup recovery could not restore a document: {}",
                                        self.status
                                    )
                                });
                            }
                        }
                    }
                    if automatic {
                        self.restore_next_startup();
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
                Reply::HistoryWarning(error) => self.warn(error),
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
        if listing_changed && !self.restoring_location {
            self.flush_locations();
        }
        while let Ok(result) = self.search.replies.try_recv() {
            if result.generation == self.search_id {
                glimpse::metrics::event(
                    if result.completed {
                        "search-complete"
                    } else {
                        "search-batch"
                    },
                    result.generation,
                    result.hits.len() as u64,
                );
                let selected = self
                    .search_ui
                    .selection
                    .and_then(|i| self.hits.get(i))
                    .map(|hit| (hit.path.clone(), hit.line));
                self.hits = std::sync::Arc::new(result.hits);
                self.search_ui.selection = selected.and_then(|(path, line)| {
                    self.hits
                        .iter()
                        .position(|hit| hit.path == path && hit.line == line)
                });
                self.search_ui.status = result.state;
                self.search_ui.running = !result.completed;
                if result.completed && self.search_ui.building {
                    self.search_ui.building = false;
                    if self
                        .search
                        .index_compatible(self.effective_scope(), &self.search_filters())
                    {
                        if let Some((panel, query)) = self.search_ui.suspended.clone() {
                            if !query.is_empty() {
                                self.start_search_value(query, panel == Panel::Text);
                            }
                        } else if matches!(self.panel, Some(Panel::Files | Panel::Text))
                            && !self.query.read(cx).value().is_empty()
                        {
                            self.start_search(cx);
                        }
                    }
                }
                self.announce_search_when_quiet(cx);
            }
        }
        if let Some(change) = self.observer.changed() {
            let refresh = change.listing_changed(&self.current);
            let ids: Vec<_> = self
                .documents
                .iter()
                .filter(|doc| doc.watched_paths().any(|path| change.affects(path)))
                .map(|doc| doc.id)
                .collect();
            if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() && (refresh || !ids.is_empty()) {
                eprintln!("WATCH_EVENT {} {refresh}", ids.len());
            }
            if let Some(error) = change.error {
                self.warn(format!(
                    "File monitoring unavailable: {error}. Use Refresh Folder to retry monitoring."
                ));
            }
            for id in ids {
                self.check_document(id);
            }
            if refresh && !self.listing_paused {
                self.refresh();
            }
        }
        if let Some(highlight) = self.syntax.take()
            && self.settings.syntax
            && let Some(doc) = self
                .documents
                .iter()
                .find(|d| d.id == highlight.id && d.generation == highlight.generation)
        {
            let canvases = theme::source_canvases(cx);
            let mut colors = HashMap::new();
            let spans = highlight
                .spans
                .into_iter()
                .map(|span| {
                    let color = *colors.entry(span.rgb).or_insert_with(|| {
                        let color =
                            glimpse::syntax::readable_rgb(span.rgb, canvases.as_flattened());
                        rgb(((color[0] as u32) << 16) | ((color[1] as u32) << 8) | color[2] as u32)
                            .into()
                    });
                    (
                        span.start..span.start + span.length,
                        HighlightStyle {
                            color: Some(color),
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
            doc.deferred_external = None;
            doc.saving = true;
            let text = glimpse::io::TextSnapshot::Revision(doc.input.read(cx).text().clone());
            self.io.send(Job::Save {
                id: doc.id,
                disk: doc.disk.clone(),
                text,
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
            text: doc.input.read(cx).value().into(),
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
    fn close(&mut self, quit: bool, tab: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
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
            .filter(|d| quit || Some(d.id) == tab)
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
                let retained = self.active.filter(|active| {
                    !decision.quit && !decision.finish && !decision.documents.contains(active)
                });
                self.active = Some(id);
                self.save(window, cx);
                if let Some(retained) = retained {
                    self.active = Some(retained);
                }
            } else if self.decision.is_none() {
                let paths = self
                    .documents
                    .iter()
                    .filter(|doc| decision.documents.contains(&doc.id) && doc.dirty(cx))
                    .map(|doc| doc.disk.path.as_path())
                    .collect::<Vec<_>>();
                let labels = paths
                    .iter()
                    .map(|path| paths::tab_label(path, paths.iter().copied()))
                    .collect::<Vec<_>>();
                let names = paths
                    .iter()
                    .map(|path| paths::display(path))
                    .collect::<Vec<_>>()
                    .join("\n");
                let finish = decision.finish;
                self.ask_decision(
                    DecisionAction::Close,
                    if finish {
                        "Finish Editing?"
                    } else {
                        "Close with unsaved changes?"
                    }
                    .into(),
                    format!("Full paths:\n{names}"),
                    vec![
                        "Cancel",
                        if finish {
                            "Save and Finish"
                        } else {
                            "Save and Close"
                        },
                        "Discard",
                    ],
                    window,
                    cx,
                );
                if let Some(view) = &self.decision {
                    view.update(cx, |view, cx| view.set_subjects(labels, cx));
                }
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
        if matches!(self.panel, Some(Panel::Files | Panel::Text))
            && self.preferences.is_none()
            && self.decision.is_none()
            && self.query.focus_handle(cx).is_focused(window)
            && key.key == "down"
            && !key.modifiers.control
            && !key.modifiers.platform
            && !self.hits.is_empty()
        {
            self.focus_results(window, cx);
            window.prevent_default();
            cx.stop_propagation();
            return;
        }
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
        glimpse::metrics::event(command, self.active.unwrap_or(0), 0);
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
                    } else if self.reading {
                        self.reading_focus.focus(window);
                    } else {
                        self.focus(window, cx);
                    }
                } else {
                    self.navigator_visible = !self.navigator_visible;
                }
            }
            "find" => {
                self.reading = false;
                self.navigation_only = false;
                self.focus(window, cx);
                if let Some(doc) = self
                    .documents
                    .iter()
                    .find(|doc| Some(doc.id) == self.active)
                {
                    doc.input
                        .update(cx, |input, cx| input.show_search(window, cx));
                }
            }
            "submit" => self.submit(window, cx),
            "dismiss" => {
                if self.panel == Some(Panel::Palette) && self.resume_search(window, cx) {
                    cx.notify();
                    return;
                }
                if let Some(token) = self.path_request.take() {
                    self.opening.remove(&token);
                }
                if self.panel == Some(Panel::Filter) {
                    self.filter.clear();
                }
                self.panel = None;
                self.search_ui.temporary = None;
                self.reset_search("Search closed");
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
                                    text: doc.input.read(cx).value().into(),
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
                    "Warnings".into(),
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
                PreparedInput::new(""),
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
                    directories: command == "folder",
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
            "line" => {
                self.reading = false;
                self.navigation_only = false;
                self.panel(Panel::Line, window, cx);
            }
            "parent" => {
                if let Some(path) = self.current.parent().map(|p| p.to_owned()) {
                    self.navigate(path);
                }
            }
            "home" => {
                if let Some(home) = std::env::home_dir() {
                    self.navigate(home);
                } else {
                    self.ask_decision(
                        DecisionAction::Notice,
                        "Home folder is unavailable".into(),
                        "Choose another folder from Locations or enter its path.".into(),
                        vec!["Close"],
                        window,
                        cx,
                    );
                }
            }
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
            "refresh" => {
                self.observer.rearm();
                self.refresh();
            }
            "stop-listing" => {
                self.listing_paused = true;
                self.directories.retain(&[]);
                self.listing_id = 0;
                self.listing_done = true;
                self.listing_waiter = None;
                self.listing_buffer.clear();
                self.listing_error =
                    Some("Folder reading was cancelled. Refresh to read again.".into());
                if !self.ready {
                    self.startup_error.get_or_insert_with(|| {
                        format!(
                            "Folder reading was cancelled: {}",
                            paths::display(&self.current)
                        )
                    });
                }
            }
            "folder-error" => {
                if let Some(error) = &self.listing_error {
                    self.ask_decision(DecisionAction::FolderError, "Folder listing incomplete".into(), format!("Folder: {}\n{error}\nListed entries may be incomplete or from a previous refresh. Retry reads this folder again.", paths::display(&self.current)), vec!["Close", "Retry"], window, cx);
                }
            }
            "refresh-volumes" => {
                self.io.send(Job::Volumes);
                self.status =
                    "Refreshing mounted locations; reopen Locations for the result".into();
            }
            "indent" | "outdent" => {
                if self.reading {
                    return;
                }
                self.focus(window, cx);
                let action: Box<dyn Action> = if command == "indent" {
                    Box::new(gpui_component::input::Indent)
                } else {
                    Box::new(gpui_component::input::Outdent)
                };
                window.dispatch_action(action, cx);
            }
            "read" => {
                self.reading = !self.reading;
                self.navigation_only = false;
                if self.reading {
                    self.reading_focus.focus(window);
                } else {
                    if let Some(doc) = self
                        .documents
                        .iter()
                        .find(|doc| Some(doc.id) == self.active)
                    {
                        doc.input.read(cx).focus_handle(cx).focus(window);
                    }
                }
                self.prepare_inactive_reading(window, cx);
            }
            "reading-retry" => {
                self.io.cancel_reading();
                self.reading_view = None;
                self.prepare_inactive_reading(window, cx);
            }
            "reading-notices" => self.reading_notices(window, cx),
            "keep-tab" => {
                if let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|doc| Some(doc.id) == self.active)
                {
                    doc.preview = false;
                }
            }
            "search-here" => {
                if !matches!(self.panel, Some(Panel::Files | Panel::Text)) {
                    self.panel(Panel::Files, window, cx);
                }
                self.search_ui.temporary = Some(self.current.clone());
                self.reset_search("Temporary search folder selected; press Enter to search");
            }
            "index-folder" => {
                if !matches!(self.panel, Some(Panel::Files | Panel::Text)) {
                    self.panel(Panel::Files, window, cx);
                }
                self.build_index();
            }
            "remove-index" => {
                self.search.clear_index();
                self.reset_search("Filename index removed; press Enter for an on-demand search");
            }
            "stop-search" => self.reset_search("Cancelled — displayed results were cleared"),
            "focus-results" => self.focus_results(window, cx),
            "search-details" => self.search_details(window, cx),
            "scope" => {
                self.scope = if self.scope.is_some() {
                    None
                } else {
                    Some(self.current.clone())
                };
                self.search_ui.temporary = None;
                self.reset_search("Search scope changed; press Enter to search");
            }
            "close" => self.close(false, self.active, window, cx),
            "quit" => self.close(true, None, window, cx),
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
                } else if let Some(path) = self
                    .search_ui
                    .picker_selection
                    .clone()
                    .or_else(|| self.selected_path())
                {
                    path
                } else {
                    self.status = "Select an existing file before choosing".into();
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
                    if let Err(error) = platform::open_path(&path) {
                        self.status = error;
                    }
                }
            }
            "go-scope" => {
                let scope = self.effective_scope().clone();
                self.navigate(scope);
                self.navigation_only = true;
                window.focus(&self.nav_focus);
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
                    self.filter.clear();
                    if self.panel == Some(Panel::Filter) {
                        self.panel = None;
                    }
                    self.reveal_pending = Some(path.clone());
                    self.selection = Some(path);
                    self.navigator_visible = true;
                    self.navigation_only = true;
                    window.focus(&self.nav_focus);
                }
            }
            "copy-path" | "copy-relative" => {
                let path = self.selected_path().unwrap_or_else(|| self.current.clone());
                let scope = self.effective_scope();
                let value = if command == "copy-relative" {
                    path.strip_prefix(scope)
                        .map_err(|_| "Selected path is outside the search folder")
                } else {
                    Ok(path.as_path())
                };
                match value {
                    Ok(path) => {
                        let value = paths::entry(path);
                        let encoded = path.to_str() != Some(value.as_str());
                        cx.write_to_clipboard(ClipboardItem::new_string(value));
                        self.status = if encoded {
                            "Copied a lossless escaped path; paste it into Location to open it"
                                .into()
                        } else {
                            "Copied path".into()
                        };
                    }
                    Err(error) => self.status = error.into(),
                }
            }
            _ => self.status = format!("Command not yet available in the GPUI port: {command}"),
        }
        cx.notify();
    }
    fn command_enabled(&self, command: &str) -> bool {
        let document = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active);
        if self.request.pick
            && matches!(
                command,
                "new"
                    | "save"
                    | "save-as"
                    | "save-copy"
                    | "named-file"
                    | "new-folder"
                    | "rename"
                    | "trash"
                    | "settings"
                    | "wrap"
                    | "indent"
                    | "outdent"
                    | "recovery"
                    | "pin"
            )
        {
            return false;
        }
        match command {
            "save" => document.is_some_and(|doc| !doc.saving && doc.external.is_none()),
            "save-as" | "save-copy" => {
                document.is_some_and(|doc| !doc.saving) && !self.destination_busy
            }
            "find" | "line" | "read" | "close" | "keep-tab" => document.is_some(),
            "indent" | "outdent" => document.is_some() && !self.reading,
            "reload" | "reveal" => {
                document.is_some_and(|doc| !doc.disk.target.as_os_str().is_empty() && !doc.saving)
            }
            "compare" | "overwrite" => {
                document.is_some_and(|doc| doc.external.is_some() && !doc.saving)
            }
            "back" => !self.history.is_empty(),
            "forward" => !self.future.is_empty(),
            "parent" => self.current.parent().is_some(),
            "finish" | "abort" => self.request.wait && !self.finished,
            "choose" | "cancel" => self.request.pick,
            _ => true,
        }
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
            .ghost()
            .disabled(!self.command_enabled(command))
            .when(
                command == "save"
                    && self.command_enabled(command)
                    && self
                        .documents
                        .iter()
                        .any(|doc| Some(doc.id) == self.active && doc.dirty(cx)),
                |button| button.primary(),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }
}
impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_reading(window, cx);
        let title = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
            .map(|doc| {
                format!(
                    "{}{} — Glimpse",
                    paths::tab_label(
                        &doc.disk.path,
                        self.documents.iter().map(|doc| doc.disk.path.as_path())
                    ),
                    if doc.dirty(cx) { " •" } else { "" }
                )
            })
            .unwrap_or_else(|| "Glimpse".into());
        if self.window_title != title {
            window.set_window_title(&title);
            self.window_title = title;
        }
        let narrow = self.narrow(window);
        let short = narrow && window.viewport_size().height < px(280. * self.settings.ui_scale);
        let compact_document =
            short && !self.navigation_only && self.active.is_some() && !self.request.pick;
        let source_visible = !self.reading && (!narrow || !self.navigation_only);
        self.report_startup_ready(source_visible, window, cx);
        if !self.listing_done && self.listing_waiter.is_none() {
            let id = self.listing_id;
            self.listing_waiter = Some(cx.spawn(async move |this, cx| {
                Timer::after(std::time::Duration::from_secs(1)).await;
                let _ = this.update(cx, |this, cx| {
                    if !this.listing_done && this.listing_id == id {
                        this.listing_stalled = true;
                        cx.notify();
                    }
                });
            }));
        }
        for doc in &self.documents {
            if !source_visible || Some(doc.id) != self.active {
                doc.input
                    .update(cx, |input, _| input.release_accessibility_cache());
            }
        }
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .p(px(8.))
            .when(short, |toolbar| toolbar.py(px(3.)))
            .bg(cx.theme().muted);
        if narrow {
            toolbar = toolbar
                .child(self.button(
                    if self.navigation_only && self.active.is_some() && self.reading {
                        "Reading"
                    } else if self.navigation_only && self.active.is_some() {
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
                .child(self.locations_menu(cx))
                .child(self.button(
                    if self.navigator_visible {
                        "Hide files"
                    } else {
                        "Show files"
                    },
                    "toggle-navigator",
                    cx,
                ))
                .child(self.button("Open", "open", cx))
                .child(self.button("Filter folder", "filter", cx))
                .child(self.button("Find files", "find-files", cx))
                .child(self.button("Find text", "find-text", cx))
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
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    if !event.modifiers.shift {
                        let ids: Vec<_> = this
                            .documents
                            .iter()
                            .filter(|doc| doc.deferred_external.is_some())
                            .map(|doc| doc.id)
                            .collect();
                        for id in ids {
                            this.apply_external(id, window, cx);
                        }
                    }
                }),
            )
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
            .when(
                !compact_document || self.panel == Some(Panel::Path),
                |root| root.child(self.path_bar(short, window, cx)),
            )
            .when(self.request.pick, |root| {
                let candidate = if self.request.directory {
                    Some(self.current.clone())
                } else {
                    self.search_ui
                        .picker_selection
                        .clone()
                        .or_else(|| self.selected_path())
                };
                let label = candidate.map_or_else(
                    || "Select a file, then Choose".into(),
                    |path| format!("Selection: {}", paths::display(&path)),
                );
                root.child(accessible(
                    "picker-selection",
                    div().px_2().text_sm().truncate().child(label.clone()),
                    move |_, _, _| {
                        let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
                        node.set_label(label.clone());
                        node.set_live(gpui::accesskit::Live::Polite);
                        AccessibilityInfo::new(node)
                    },
                ))
            });
        if self.open_failure.is_some() {
            root = root.child(self.open_failure_view(cx));
        }
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
                        row.child(self.button(
                            if matches!(panel, Panel::Files | Panel::Text) {
                                "Search"
                            } else {
                                "Apply"
                            },
                            "submit",
                            cx,
                        ))
                    })
                    .child(self.button("Close", "dismiss", cx)),
            );
            if matches!(panel, Panel::Files | Panel::Text) {
                root = root.child(self.search_view(window, cx));
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
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.palette_command(command.id, window, cx)
                                        },
                                    ))
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

        let visible = self.visible_rows();
        let entries = self.entries.clone();
        let scroll = self
            .scrolls
            .entry(self.current.clone())
            .or_default()
            .clone();
        if self.listing_done
            && let Some(path) = self.reveal_pending.take()
            && let Some(index) = visible
                .iter()
                .position(|&index| entries[index].path == path)
        {
            scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
        let navigator = div()
            .id("navigator")
            .track_focus(&self.nav_focus)
            .w_full()
            .h_full()
            .bg(cx.theme().sidebar)
            .border_2()
            .border_color(cx.theme().sidebar)
            .focus(|style| style.border_color(cx.theme().foreground))
            .flex()
            .flex_col()
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
            .when(!self.listing_done || self.listing_error.is_some() || visible.is_empty(), |navigator| {
                let message = if let Some(error) = &self.listing_error {
                    format!("Folder unavailable or partial: {error}. Existing rows may be from the previous refresh.")
                } else if self.listing_stalled {
                    "Still reading this folder. You can stop, navigate elsewhere or keep editing.".into()
                } else if !self.listing_done {
                    if self.entries.is_empty() { "Loading folder…".into() } else { "Refreshing folder…".into() }
                } else if self.entries.is_empty() {
                    "This folder is empty".into()
                } else {
                    "No filenames match the filter".into()
                };
                navigator.child(
                    div().p_2().text_sm()
                        .child(div().id("listing-state").truncate().child(message.clone())
                            .tooltip(move |window,cx| gpui_component::tooltip::Tooltip::new(message.clone()).build(window,cx)))
                        .child(self.button("Refresh", "refresh", cx))
                        .when(!self.listing_done, |row| row.child(self.button("Stop Reading", "stop-listing", cx)))
                        .when(self.listing_error.is_some(), |row| row.child(self.button("Details…", "folder-error", cx)))
                )
            })
            .when(!visible.is_empty(), |navigator| navigator.child(
                uniform_list(
                    "entries",
                    visible.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|i| {
                                let entry = &entries[visible[i]];
                                let path = entry.path.clone();
                                let selected = this.selection.as_ref() == Some(&path);
                                let tooltip = paths::display(&path);
                                let row=div()
                                    .id(("entry", i))
                                    .h(px(if this.settings.compact { 28. } else { 32. }
                                        * this.settings.ui_scale))
                                    .border_l(px(3. * this.settings.ui_scale))
                                    .border_color(if selected {
                                        cx.theme().foreground
                                    } else {
                                        gpui::transparent_black()
                                    })
                                    .px_3()
                                    .py_1()
                                    .bg(if selected {
                                        cx.theme().selection
                                    } else {
                                        cx.theme().sidebar
                                    })
                                    .truncate()
                                    .tooltip(move |window,cx| gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window,cx))
                                    .cursor_pointer()
                                    .child(format!(
                                        "{} {}",
                                        if entry.symlink { "↗" } else if entry.directory { "▸" } else { "·" },
                                        paths::name(&path)
                                    ))
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, _, cx| {
                                            this.selection = Some(path.clone());
                                            this.search_ui.picker_selection = Some(path.clone());
                                            this.open(path.clone(), event.click_count() < 2, None);
                                            cx.notify();
                                        },
                                    ));
                                this.navigation_row(entry,(i,visible.len()),selected,row,cx)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0()
                .track_scroll(scroll),
            ));
        let navigator = accessibility::collection(
            "navigator-list",
            format!("Files in {}", paths::display(&self.current)),
            navigator,
            self.nav_focus.clone(),
        );
        let tabs = div()
            .id("tabs")
            .w_full()
            .flex()
            .gap_1()
            .p(px(4.))
            .when(short, |tabs| tabs.py_0())
            .overflow_x_scroll()
            .flex_shrink_0()
            .track_scroll(&self.tab_scroll)
            .children(self.documents.iter().map(|doc| {
                let id = doc.id;
                let name = paths::tab_label(
                    &doc.disk.path,
                    self.documents.iter().map(|doc| doc.disk.path.as_path()),
                );
                let title = format!(
                    "{name}{}{}{}",
                    if doc.dirty(cx) { " •" } else { "" },
                    if doc.preview { " (preview)" } else { "" },
                    if doc.external.is_some() {
                        " (conflict)"
                    } else {
                        ""
                    }
                );
                div()
                    .track_focus(&doc.tab_focus)
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .child(
                        Button::new(("tab", id))
                            .accessibility_role(gpui::accesskit::Role::Tab)
                            .label(title)
                            .tooltip(paths::display(&doc.disk.path))
                            .small()
                            .compact()
                            .selected(Some(id) == self.active)
                            .when(doc.preview, |tab| tab.italic_label())
                            .when(Some(id) == self.active, |tab| tab.primary())
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
                    .when(!self.request.pick, |tab| {
                        tab.child(
                            Button::new(("close-tab", id))
                                .icon(IconName::Close)
                                .small()
                                .ghost()
                                .tooltip(format!("Close {name}"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close(false, Some(id), window, cx);
                                })),
                        )
                    })
            }));
        let tabs = accessible("open-documents", tabs, |_, _, _| {
            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::TabList);
            node.set_label("Open documents");
            AccessibilityInfo::new(node)
        });
        let tabs = if compact_document && self.reading {
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .child(div().flex_1().min_w_0().child(tabs))
                .child(self.button("Edit Source", "read", cx).flex_shrink_0())
                .into_any_element()
        } else {
            tabs.into_any_element()
        };
        let mut content = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs);
        if !self.request.pick && !short {
            let actions = div().flex().flex_wrap().gap_2().p_1();
            content = content.child(if narrow {
                actions
                    .child(self.button("Save", "save", cx))
                    .child(self.button(
                        if self.reading { "Edit Source" } else { "Read" },
                        "read",
                        cx,
                    ))
            } else {
                actions
                    .child(self.button("New", "new", cx))
                    .child(self.button("Save", "save", cx))
                    .child(self.button("Save As", "save-as", cx))
                    .child(self.button("Save Copy", "save-copy", cx))
                    .child(self.button(
                        if self.reading { "Edit Source" } else { "Read" },
                        "read",
                        cx,
                    ))
                    .child(self.button("Go to line", "line", cx))
                    .child(self.button("Reload disk", "reload", cx))
            });
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            if !doc.disk.target.as_os_str().is_empty()
                && !doc.disk.path.starts_with(self.effective_scope())
            {
                content = content.child(
                    div().px_2().py_1().text_sm().child(
                        self.command_button(
                            "document-search-scope",
                            format!(
                                "Outside search folder: {}",
                                paths::name(self.effective_scope())
                            ),
                            "search-details",
                            cx,
                        )
                        .tooltip(paths::display(self.effective_scope())),
                    ),
                );
            }
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
                        .child({
                            let message = match doc.failure_action {
                                FailureAction::Save => "Last save failed",
                                FailureAction::Reload => "Reload stopped",
                                FailureAction::Check => "File unavailable",
                                FailureAction::Choose(_) => "Destination unavailable",
                            };
                            accessibility::message(
                                "operation-failure",
                                message.into(),
                                div().child(message),
                            )
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
                        .child(accessibility::message(
                            "external-conflict",
                            "Changed on disk. Local text retained; ordinary Save is blocked."
                                .into(),
                            div().child(
                                "Changed on disk. Local text retained; ordinary Save is blocked.",
                            ),
                        ))
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
                content = content.child(self.reading_element(narrow, short, cx));
            } else {
                content = content.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(if self.settings.proportional && doc.grammar(cx).is_none() {
                            cx.theme().font_family.clone()
                        } else {
                            cx.theme().mono_font_family.clone()
                        })
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
        let has_status = !self.status.is_empty() || self.font_warning.is_some();
        let stacked_footer = narrow && !short && (has_status || !self.warnings.is_empty());
        let footer_height = if stacked_footer {
            48.
        } else if short && self.warnings.is_empty() {
            20.
        } else if narrow {
            28.
        } else {
            36.
        };
        root.child(div().flex_1().min_h_0().child(body))
            .child(
                div()
                    .h(px(footer_height * self.settings.ui_scale))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12. * self.settings.ui_scale))
                    .when(stacked_footer, |footer| {
                        footer.flex_col().items_start().justify_center().gap_0()
                    })
                    .when(!self.warnings.is_empty(), |footer| {
                        footer.child(self.button("Warnings…", "warnings", cx))
                    })
                    .when(
                        self.warnings.is_empty() && (!narrow || has_status),
                        |footer| {
                            let message = match &self.font_warning {
                                Some(warning) if self.status.is_empty() => warning.clone(),
                                Some(warning) => format!("{} · {warning}", self.status),
                                None => self.status.clone(),
                            };
                            footer.child(accessibility::message(
                                "application-status",
                                message.clone(),
                                div()
                                    .id("status-message")
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(message.clone())
                                    .tooltip(move |window, cx| {
                                        gpui_component::tooltip::Tooltip::new(message.clone())
                                            .build(window, cx)
                                    }),
                            ))
                        },
                    )
                    .when_some(
                        self.documents
                            .iter()
                            .find(|doc| Some(doc.id) == self.active),
                        |footer, doc| {
                            let information = doc.information(cx, self.reading);
                            footer.child(accessible(
                                "document-information",
                                div()
                                    .when(short, |info| {
                                        info.truncate().when(
                                            !self.status.is_empty()
                                                || self.font_warning.is_some()
                                                || !self.warnings.is_empty(),
                                            |info| info.max_w(rems(10.)),
                                        )
                                    })
                                    .flex_shrink_0()
                                    .child(information.clone()),
                                move |_, _, _| {
                                    let mut node =
                                        gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
                                    node.set_label(information.clone());
                                    AccessibilityInfo::new(node)
                                },
                            ))
                        },
                    ),
            )
            .children(self.preferences.clone())
            .children(self.decision.clone())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.preferences.is_some() || this.decision.is_some() {
                    return;
                }
                let key = &event.keystroke;
                if this.search_ui.focus.is_focused(window) && this.search_key(key, window, cx) {
                    return;
                }
                if this.nav_focus.is_focused(window)
                    && !key.modifiers.control
                    && !key.modifiers.platform
                {
                    let visible = this.visible_rows();
                    let rows = this.entries.clone();
                    let index = visible
                        .iter()
                        .position(|&i| Some(&rows[i].path) == this.selection.as_ref());
                    match key.key.as_str() {
                        "up" | "down" if !visible.is_empty() => {
                            let next = if key.key == "down" {
                                index.map_or(0, |i| (i + 1).min(visible.len() - 1))
                            } else {
                                index.unwrap_or(0).saturating_sub(1)
                            };
                            this.selection = Some(rows[visible[next]].path.clone());
                            this.search_ui.picker_selection = this.selection.clone();
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
                        "right" | "enter"
                            if key.key == "enter"
                                || index.is_some_and(|index| rows[visible[index]].directory) =>
                        {
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
