mod accessibility;
mod completion;
mod conflict;
mod decision;
mod destination;
mod external;
mod keyboard;
mod labels;
mod menus;
mod modal;
mod notice;
mod panel;
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
    input::{Input, InputEvent, InputState, LineType, Position, PreparedInput, Rope},
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

    fn markdown(&self) -> bool {
        self.disk
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["md", "markdown", "mdown", "mkd", "mkdn"]
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            })
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
/// Active or focused tab index, window size, and each document's label-changing states.
type TabReveal = (usize, Size<Pixels>, Vec<(u64, bool, bool, bool)>);

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
struct TrashRequest {
    path: PathBuf,
    adjacent: Option<PathBuf>,
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
    palette: panel::PaletteState,
    scope: Option<PathBuf>,
    current: PathBuf,
    launch_folder: PathBuf,
    history: Vec<PathBuf>,
    gesture: glimpse::gesture::Gesture,
    future: Vec<PathBuf>,
    nav_focus: FocusHandle,
    open_documents_requested: bool,
    main_menu_requested: bool,
    navigator_menu: Option<menus::NavigatorMenu>,
    query_error: Option<String>,
    panel_revision: u64,
    name_request: Option<u64>,
    rename_path: Option<PathBuf>,
    conflict_focus: FocusHandle,
    edit_state: std::cell::Cell<[bool; 3]>,
    reading_focus: FocusHandle,
    pane_state: Entity<panes::Divider>,
    navigation_only: bool,
    navigator_visible: bool,
    scrolls: HashMap<PathBuf, UniformListScrollHandle>,
    selection: Option<PathBuf>,
    trash_pending: Option<TrashRequest>,
    trash_needs_selection: bool,
    reveal_pending: Option<PathBuf>,
    selections: HashMap<PathBuf, PathBuf>,
    entries: std::sync::Arc<Vec<Entry>>,
    entries_revision: u64,
    visible_key: Option<(u64, String, bool)>,
    visible_entries: std::sync::Arc<Vec<usize>>,
    listing_buffer: Vec<Entry>,
    navigation_hints: HashMap<String, paths::LabelCue>,
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
    tab_widths: HashMap<u64, Pixels>,
    tab_reveal: Option<TabReveal>,
    next_id: u64,
    open_token: u64,
    interaction: u64,
    path_request: Option<u64>,
    path_completion_token: u64,
    path_completions: Option<path::Completions>,
    path_hint: Option<String>,
    path_scroll: ScrollHandle,
    panel_return: Option<FocusHandle>,
    type_ahead: (String, Option<std::time::Instant>),
    opening: HashMap<u64, OpenIntent>,
    hit_versions: HashMap<u64, (u64, u64)>,
    panel: Option<Panel>,
    query: Entity<InputState>,
    reading_documents: HashSet<u64>,
    reading_view: Option<reading::View>,
    reading_request: u64,
    memory_release: Option<Task<()>>,
    status: String,
    last_saved: Option<(u64, u64)>,
    open_failure: Option<notice::Failure>,
    notice_focus: FocusHandle,
    notice_details_focus: FocusHandle,
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
    cleanup_texts: HashMap<u64, (u64, Rope)>,
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
                if let Some(token) = this.path_request.take() {
                    this.opening.remove(&token);
                }
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
            if matches!(event, InputEvent::Change) && this.query_error.take().is_some() {
                cx.notify();
            }
            if matches!(event, InputEvent::Change) && this.panel == Some(Panel::Palette) {
                this.palette.selection = 0;
                this.palette.scroll.scroll_to_item(0);
            }
            if matches!(event, InputEvent::Change)
                && matches!(this.panel, Some(Panel::Palette | Panel::Recovery))
            {
                cx.notify();
            }
            if matches!(event, InputEvent::Change) && this.panel == Some(Panel::Filter) {
                this.filter = this.query.read(cx).value().to_string();
                // Keep the row Enter opens visible and selected while the filter narrows.
                let visible = this.visible_rows();
                let rows = this.entries.clone();
                if !visible
                    .iter()
                    .any(|&i| Some(&rows[i].path) == this.selection.as_ref())
                    && let Some(&first) = visible.first()
                {
                    this.selection = Some(rows[first].path.clone());
                    this.search_ui.picker_selection = this.selection.clone();
                    if let Some(scroll) = this.scrolls.get(&this.current) {
                        scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                }
                cx.notify();
            }
        });
        let shortcut_view = cx.weak_entity();
        let shortcut_window = window.window_handle();
        let shortcut_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == shortcut_window {
                let _ = shortcut_view.update(cx, |this: &mut Self, cx| {
                    this.interaction += 1;
                    this.route_key(event, window, cx)
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
            open_documents_requested: false,
            main_menu_requested: false,
            navigator_menu: None,
            query_error: None,
            panel_revision: 0,
            name_request: None,
            rename_path: None,
            conflict_focus: cx.focus_handle(),
            edit_state: Default::default(),
            reading_focus: cx.focus_handle().tab_stop(true),
            pane_state: cx.new(panes::Divider::new),
            navigation_only: false,
            navigator_visible: true,
            scrolls: HashMap::new(),
            selection: None,
            reveal_pending: None,
            trash_pending: None,
            trash_needs_selection: false,
            selections: HashMap::new(),
            entries: Default::default(),
            entries_revision: 0,
            visible_key: None,
            visible_entries: Default::default(),
            listing_buffer: vec![],
            navigation_hints: Default::default(),
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
            tab_widths: HashMap::new(),
            tab_reveal: None,
            next_id: 1,
            open_token,
            interaction: 0,
            path_request: None,
            path_completion_token: 0,
            path_completions: None,
            path_hint: None,
            path_scroll: ScrollHandle::new(),
            panel_return: None,
            type_ahead: (String::new(), None),
            opening,
            hit_versions: HashMap::new(),
            panel: None,
            query,
            reading_documents: HashSet::new(),
            reading_view: None,
            reading_request: 0,
            memory_release: None,
            status: String::new(),
            last_saved: None,
            open_failure: None,
            notice_focus: cx.focus_handle(),
            notice_details_focus: cx.focus_handle().tab_stop(true),
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
            palette: panel::PaletteState::new(cx),
        };
        window.focus(&this.nav_focus);
        this.refresh();
        this.subscriptions
            .push(cx.observe(&this.pane_state, |_, _, cx| cx.notify()));
        this.subscriptions
            .push(cx.observe_window_activation(window, |this, window, cx| {
                if this.reading() {
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
        // The minimum window grows with the interface scale so every scale keeps the room of
        // the 520×400 layout, capped at the display size.
        let scale = self.settings.ui_scale;
        let mut minimum = size(px(520. * scale), px(400. * scale));
        if let Some(display) = window.display(cx) {
            let available = display.bounds().size;
            minimum = size(
                minimum.width.min(available.width),
                minimum.height.min(available.height),
            );
        }
        window.set_window_min_size(minimum);
        let current = window.viewport_size();
        if current.width < minimum.width || current.height < minimum.height {
            window.resize(size(
                current.width.max(minimum.width),
                current.height.max(minimum.height),
            ));
        }
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
                decision.close_actions(window, cx);
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
        self.clear_open_failure();
        self.trash_needs_selection = false;
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
    fn path_is_open(&self, path: &Path) -> bool {
        self.documents
            .iter()
            .any(|doc| doc.disk.path.starts_with(path) || doc.disk.target.starts_with(path))
    }

    fn select_navigation(&mut self, path: PathBuf) {
        if self
            .open_failure
            .as_ref()
            .is_some_and(|failure| failure.path != path)
        {
            self.clear_open_failure();
        }
        self.selection = Some(path.clone());
        self.search_ui.picker_selection = Some(path);
        if self.trash_needs_selection && self.status == Self::TRASH_SELECTION_GUIDANCE {
            self.status.clear();
        }
        self.trash_needs_selection = false;
    }

    const TRASH_SELECTION_GUIDANCE: &str =
        "Press Space to select the item before moving it to Trash.";

    fn file_action_blocker(&self, path: &Path, needs_selection: bool) -> Option<&'static str> {
        if self.trash_pending.is_some() {
            Some("Wait for the move to Trash to finish.")
        } else if needs_selection && self.trash_needs_selection {
            Some(Self::TRASH_SELECTION_GUIDANCE)
        } else if self.path_is_open(path) {
            Some("Close open documents in this item first.")
        } else {
            None
        }
    }

    fn move_to_trash(&mut self, path: PathBuf, needs_selection: bool) {
        if self.request.pick {
            return;
        }
        if let Some(reason) = self.file_action_blocker(&path, needs_selection) {
            self.status = reason.into();
        } else {
            self.trash_selection(path);
        }
    }

    fn navigation_row_height(&self) -> Pixels {
        px(if self.settings.compact { 28. } else { 32. } * self.settings.ui_scale)
    }

    fn trash_selection(&mut self, path: PathBuf) {
        let visible = self.visible_rows();
        let adjacent = visible
            .iter()
            .position(|&index| self.entries[index].path == path)
            .and_then(|index| {
                visible
                    .get(index + 1)
                    .or_else(|| index.checked_sub(1).and_then(|index| visible.get(index)))
            })
            .map(|&index| self.entries[index].path.clone());
        self.trash_pending = Some(TrashRequest {
            path: path.clone(),
            adjacent,
        });
        self.trash_needs_selection = true;
        self.type_ahead.0.clear();
        self.io.send(Job::Trash(path));
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
        if activate {
            self.clear_open_failure();
        }
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
            input.set_accessibility_label(format!("Source: {}", paths::name(&disk.path)), cx);
            input.set_accessibility_description(paths::display(&disk.path), cx);
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
                    !this.reading() && (!this.narrow(window) || !this.navigation_only);
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
            self.set_reading(false);
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
    /// Whether the active document is shown in Reading; each document keeps its own view.
    fn reading(&self) -> bool {
        self.active
            .is_some_and(|id| self.reading_documents.contains(&id))
    }
    fn set_reading(&mut self, reading: bool) {
        if let Some(id) = self.active {
            if reading {
                self.reading_documents.insert(id);
            } else {
                self.reading_documents.remove(&id);
            }
        }
    }
    /// Whether a focus target is still on screen: hidden documents' editors and tabs, and the
    /// Reading view of a document shown as source, are not.
    fn shown(&self, handle: &FocusHandle, cx: &App) -> bool {
        let hidden_document = self.documents.iter().any(|doc| {
            Some(doc.id) != self.active
                && (&doc.tab_focus == handle || &doc.input.focus_handle(cx) == handle)
        });
        let hidden_reading = handle == &self.reading_focus && !self.reading();
        let hidden_source = self.reading()
            && self
                .documents
                .iter()
                .any(|doc| Some(doc.id) == self.active && &doc.input.focus_handle(cx) == handle);
        !(hidden_document || hidden_reading || hidden_source)
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
            if self.reading() {
                window.focus(&self.reading_focus);
            } else {
                doc.input.update(cx, |input, cx| input.focus(window, cx));
            }
        } else {
            window.focus(&self.nav_focus);
        }
    }
    fn panel(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_none() {
            self.panel_return = window.focused(cx);
        }
        self.path_completions = None;
        self.path_hint = None;
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
        if let Some(token) = self.path_request.take() {
            self.opening.remove(&token);
        }
        self.panel = Some(panel);
        self.panel_revision += 1;
        self.query_error = None;
        self.rename_path = None;
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
                    Panel::NewFile | Panel::NewFolder => "One filename",
                    Panel::Rename => "New Name",
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
                let matches = self.palette_matches(cx);
                if let Some(id) = matches
                    .get(self.palette.selection.min(matches.len().saturating_sub(1)))
                    .copied()
                {
                    self.palette_command(id, window, cx);
                }
            }
            Some(Panel::Path) => match paths::parse_entry(&value, &self.current) {
                Ok(path) => {
                    if let Some(old) = self.path_request.take() {
                        self.opening.remove(&old);
                    }
                    self.query_error = None;
                    self.path_completions = None;
                    self.path_hint = None;
                    self.path_request = self.open(path, false, None);
                }
                Err(error) => self.query_error = Some(error),
            },
            Some(Panel::Files)
                if self.search_ui.started
                    && !self.search_ui.running
                    && self.search_ui.query == value =>
            {
                if let Some(index) = self.search_ui.selection {
                    self.open_hit(index, cx);
                }
            }
            Some(Panel::Files) if self.search_ui.running && self.search_ui.query == value => {}
            Some(Panel::Files | Panel::Text) => self.start_search(cx),
            Some(Panel::Filter) => {
                let visible = self.visible_rows();
                let rows = self.entries.clone();
                let path = visible
                    .iter()
                    .map(|&i| &rows[i].path)
                    .find(|path| Some(*path) == self.selection.as_ref())
                    .or_else(|| visible.first().map(|&i| &rows[i].path))
                    .cloned();
                if let Some(path) = path {
                    self.select_navigation(path.clone());
                    self.open(path, false, None);
                }
            }
            Some(Panel::NewFile | Panel::NewFolder | Panel::Rename) => {
                if self.name_request.is_some() {
                    return;
                }
                let ticket = self.panel_revision;
                let job = if value.is_empty() {
                    self.query_error = Some("Enter a name.".into());
                    None
                } else if !paths::single_name(&value) {
                    self.query_error = Some("Enter a single name without a folder path.".into());
                    None
                } else if self.panel == Some(Panel::Rename) {
                    self.rename_path.clone().and_then(|source| {
                        let destination = source.with_file_name(&value);
                        if self.path_is_open(&source) {
                            self.query_error =
                                Some("Close documents in this item before renaming it.".into());
                            None
                        } else if destination == source {
                            self.command("dismiss", window, cx);
                            None
                        } else {
                            Some(Job::Rename(ticket, source, destination))
                        }
                    })
                } else {
                    Some(Job::Create(
                        ticket,
                        self.current.join(&value),
                        self.panel == Some(Panel::NewFolder),
                    ))
                };
                if let Some(job) = job {
                    self.query_error = None;
                    self.name_request = Some(ticket);
                    self.io.send(job);
                }
            }
            Some(Panel::Line) => {
                if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
                    let lines = doc.input.read(cx).text().len_lines(LineType::LF);
                    match value.trim().parse::<u32>() {
                        Ok(line) if (1..=lines).contains(&(line as usize)) => {
                            self.panel = None;
                            self.panel_return = None;
                            let input = doc.input.clone();
                            self.set_reading(false);
                            self.navigation_only = false;
                            input.update(cx, |input, cx| {
                                input.set_cursor_position(Position::new(line - 1, 0), window, cx);
                            });
                        }
                        _ => {
                            let message = if lines == 1 {
                                "Enter 1. This file has one line.".into()
                            } else {
                                format!("Enter a whole number from 1 to {lines}.")
                            };
                            self.query_error = Some(message);
                            self.query.update(cx, |input, cx| input.focus(window, cx));
                        }
                    }
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
        let mut navigation_hints = HashMap::new();
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
                navigation_hints = batch.filename_hints;
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
                self.navigation_hints = navigation_hints;
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
                        let search_generation =
                            self.search_ui.file_opens.remove(&token).or_else(|| {
                                self.hit_versions
                                    .remove(&token)
                                    .map(|(_, generation)| generation)
                            });
                        if let Some(generation) = search_generation {
                            if generation == self.search_id {
                                self.search_ui.status = "This result is now a folder. Search again for current results.".into();
                            }
                        } else if !self.ready && self.request.files.contains(&path) {
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
                    let file_search_generation = self.search_ui.file_opens.remove(&token);
                    let search_generation = expected
                        .map(|(_, generation)| generation)
                        .or(file_search_generation);
                    if let Some(OpenIntent {
                        preview,
                        position,
                        interaction,
                    }) = self.opening.remove(&token)
                    {
                        let activate = interaction == self.interaction
                            && search_generation.is_none_or(|generation| {
                                token == self.open_token
                                    && generation == self.search_id
                                    && self.panel
                                        == Some(if file_search_generation.is_some() {
                                            Panel::Files
                                        } else {
                                            Panel::Text
                                        })
                            })
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
                                if let Some((expected, search_generation)) = expected {
                                    let dirty = self
                                        .documents
                                        .iter()
                                        .any(|doc| doc.disk.same_file(&disk) && doc.dirty(cx));
                                    let changed = expected
                                        != search::fingerprint_body(
                                            text.value().as_bytes(),
                                            disk.bom,
                                        );
                                    if dirty || changed {
                                        if search_generation == self.search_id {
                                            self.search_ui.status = if dirty {
                                                "Failed to open result. Save edits, then search again."
                                            } else {
                                                "Failed to open result. File changed. Search again."
                                            }
                                            .into();
                                        }
                                        continue;
                                    }
                                }
                                if activate {
                                    if file_search_generation.is_some() && !self.request.pick {
                                        self.panel = None;
                                        self.panel_return = None;
                                        self.search_ui.temporary = None;
                                        self.reset_search("Search closed");
                                    } else if search_generation.is_some() {
                                        self.search_ui.collapsed = self.search_ui.takeover;
                                    } else if from_path {
                                        self.panel = None;
                                        self.panel_return = None;
                                    }
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
                                if search_generation == Some(self.search_id) {
                                    self.search_ui.status = format!("Failed to open result: {e}");
                                }
                                if from_path && activate {
                                    self.query_error =
                                        Some(format!("Could not open this path. {e}"));
                                } else if activate {
                                    self.record_open_failure(path, e, window, cx);
                                }
                            }
                        }
                    }
                }
                Reply::Cleared(id, generation, result) => {
                    let _work = gpui::work_trace::Span::new("close_cleanup_acknowledge");
                    if let Some((pending_generation, text)) = self.cleanup_texts.get(&id)
                        && *pending_generation == generation
                        && let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == id)
                        && {
                            let input = doc.input.read(cx);
                            !input.has_text_revision(text) && input.text() != text
                        }
                    {
                        doc.generation += 1;
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
                                self.reading_documents.remove(&id);
                                if self.active == Some(id) {
                                    self.active = self.documents.last().map(|d| d.id);
                                }
                            }
                            self.complete_request();
                            if self.reading() {
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
                                self.last_saved = Some((id, doc.generation));
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
                                        format!("Source: {}", paths::name(&doc.disk.path)),
                                        cx,
                                    );
                                    input.set_accessibility_description(
                                        paths::display(&doc.disk.path),
                                        cx,
                                    );
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
                                self.last_saved = Some((id, doc.generation));
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
                Reply::NamedChange(ticket, path, result) => {
                    self.name_request = None;
                    let current_form = self.panel_revision == ticket
                        && matches!(
                            self.panel,
                            Some(Panel::NewFile | Panel::NewFolder | Panel::Rename)
                        );
                    match result {
                        Ok(()) => {
                            if path.parent() == Some(self.current.as_path()) {
                                self.selection = Some(path.clone());
                                self.reveal_pending = Some(path);
                                self.refresh();
                            }
                            if current_form {
                                self.panel = None;
                                self.panel_return = None;
                                self.rename_path = None;
                                window.focus(&self.nav_focus);
                            }
                        }
                        Err(error) if current_form => {
                            self.query_error = Some(error);
                            self.query.update(cx, |input, cx| input.focus(window, cx));
                        }
                        Err(error) => self.status = error,
                    }
                }
                Reply::Trashed(path, result) => {
                    if let Some(request) = self.trash_pending.take() {
                        match result {
                            Ok(()) => {
                                if path.parent() == Some(self.current.as_path()) {
                                    if self.selection.as_ref() == Some(&path) {
                                        self.selection = request.adjacent;
                                        self.search_ui.picker_selection = self.selection.clone();
                                    }
                                    self.refresh();
                                }
                                self.status = format!("Moved to Trash: {}", paths::name(&path));
                            }
                            Err(error) => {
                                self.trash_needs_selection = false;
                                self.status = format!(
                                    "Could not move to Trash. {error}\n{}",
                                    paths::name(&path)
                                );
                            }
                        }
                    }
                }
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
                self.search_ui.duplicate_parents = std::sync::Arc::new(result.duplicate_parents);
                self.search_ui.filename_hints = std::sync::Arc::new(result.filename_hints);
                self.hits = std::sync::Arc::new(result.hits);
                self.search_ui.selection = selected
                    .and_then(|(path, line)| {
                        self.hits
                            .iter()
                            .position(|hit| hit.path == path && hit.line == line)
                    })
                    .or_else(|| {
                        (result.completed
                            && self.panel == Some(Panel::Files)
                            && !self.hits.is_empty())
                        .then_some(0)
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
            let text = doc.input.update(cx, |input, _| {
                input.begin_undo_group();
                glimpse::io::TextSnapshot::Revision(input.text().clone())
            });
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
                let subjects = decision::Decision::unsaved_subjects(&paths);
                let details = paths
                    .iter()
                    .map(|path| paths::display(path))
                    .collect::<Vec<_>>()
                    .join("\n");
                let finish = decision.finish;
                let title = match (finish, paths.len()) {
                    (true, 0..=1) => "Finish Editing?".into(),
                    (false, 0..=1) => "Close with unsaved changes?".into(),
                    (true, count) => format!("Finish editing {count} unsaved documents?"),
                    (false, count) => format!("Close {count} unsaved documents?"),
                };
                self.ask_decision(
                    DecisionAction::Close,
                    title,
                    details,
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
                    view.update(cx, |view, cx| view.set_subjects(subjects, window, cx));
                }
            }
        } else {
            self.commit_close(false, cx);
        }
        cx.notify();
    }
    fn commit_close(&mut self, discard: bool, cx: &mut Context<Self>) {
        let _work = gpui::work_trace::Span::new("close_request");
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
                self.cleanup_texts
                    .insert(doc.id, (doc.generation, doc.input.read(cx).text().clone()));
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
        glimpse::metrics::event(command, self.active.unwrap_or(0), 0);
        if self.request.pick
            && matches!(
                command,
                "named-file"
                    | "new-folder"
                    | "rename"
                    | "keep-tab"
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
            "open-documents" if self.documents.len() > 1 && !self.request.pick => {
                self.navigation_only = false;
                self.open_documents_requested = true;
            }
            "open-documents" => {}
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
            "rename" => {
                if let Some(path) = self.selected_path() {
                    if let Some(reason) = self.file_action_blocker(&path, false) {
                        self.status = reason.into();
                        cx.notify();
                        return;
                    }
                    self.panel(Panel::Rename, window, cx);
                    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                        let directory = self
                            .entries
                            .iter()
                            .any(|entry| entry.path == path && entry.directory);
                        let stem = if directory {
                            name
                        } else {
                            path.file_stem()
                                .and_then(|stem| stem.to_str())
                                .unwrap_or(name)
                        };
                        self.query.update(cx, |input, cx| {
                            input.set_value(name.to_owned(), window, cx);
                            input.set_cursor_offset_unfocused(0, cx);
                            input.select_to(stem.len(), cx);
                        });
                    }
                    self.rename_path = Some(path);
                } else {
                    self.status = "Select a file or folder to rename.".into();
                }
            }
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
                    } else if self.reading() {
                        self.reading_focus.focus(window);
                    } else {
                        self.focus(window, cx);
                    }
                } else {
                    self.navigator_visible = !self.navigator_visible;
                }
            }
            "find" => {
                self.set_reading(false);
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
            "submit" if self.panel == Some(Panel::Files) => self.start_search(cx),
            "submit" => self.submit(window, cx),
            "next-document" | "previous-document" => {
                if let Some(index) = self
                    .documents
                    .iter()
                    .position(|doc| Some(doc.id) == self.active)
                {
                    let count = self.documents.len();
                    let next = if command == "next-document" {
                        (index + 1) % count
                    } else {
                        (index + count - 1) % count
                    };
                    let id = self.documents[next].id;
                    self.select_tab(id, false, window, cx);
                }
            }
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
                // Focus returns to the control that opened the panel only when it is still in
                // the panel; after opening a result the user is already where they work.
                let inside = self.query.focus_handle(cx).contains_focused(window, cx)
                    || self.search_ui.focus.contains_focused(window, cx)
                    || self.palette.overlay.contains_focused(window, cx)
                    || self.palette.focus.contains_focused(window, cx)
                    || (self.panel == Some(Panel::Filter) && self.nav_focus.is_focused(window));
                self.panel = None;
                self.query_error = None;
                self.path_hint = None;
                self.search_ui.temporary = None;
                self.reset_search("Search closed");
                match self.panel_return.take() {
                    Some(invoker) if inside && self.shown(&invoker, cx) => window.focus(&invoker),
                    _ if inside => self.focus(window, cx),
                    _ => {}
                }
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
            "filter" => {
                self.navigator_visible = true;
                if self.narrow(window) {
                    self.navigation_only = true;
                }
                self.panel(Panel::Filter, window, cx);
            }
            "find-files" => self.panel(Panel::Files, window, cx),
            "find-text" => self.panel(Panel::Text, window, cx),
            "commands" => self.panel(Panel::Palette, window, cx),
            "settings" => self.open_preferences(self.settings.clone(), window, cx),
            "recovery" => self.panel(Panel::Recovery, window, cx),
            "refresh-recovery" => {
                self.io.send(Job::Inventory);
            }
            "line" => {
                self.set_reading(false);
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
            "undo" | "redo" | "cut" | "copy" | "paste" | "select-all" => {
                if self.reading() {
                    return;
                }
                self.focus(window, cx);
                let action: Box<dyn Action> = match command {
                    "undo" => Box::new(gpui_component::input::Undo),
                    "redo" => Box::new(gpui_component::input::Redo),
                    "cut" => Box::new(gpui_component::input::Cut),
                    "copy" => Box::new(gpui_component::input::Copy),
                    "paste" => Box::new(gpui_component::input::Paste),
                    _ => Box::new(gpui_component::input::SelectAll),
                };
                window.dispatch_action(action, cx);
            }
            "indent" | "outdent" => {
                if self.reading() {
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
                let reading = !self.reading();
                self.set_reading(reading);
                self.navigation_only = false;
                if self.reading() {
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
                let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|doc| doc.preview && Some(doc.id) == self.active)
                else {
                    return;
                };
                doc.preview = false;
                if !self.navigation_only {
                    if self.reading() {
                        self.reading_focus.focus(window);
                    } else {
                        self.focus(window, cx);
                    }
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
                    self.move_to_trash(path, true);
                } else {
                    self.status = "Select a file or folder to move to Trash.".into();
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
                    | "keep-tab"
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
            "open-documents" | "next-document" | "previous-document" => {
                self.documents.len() > 1 && !self.request.pick
            }
            "save" => document.is_some_and(|doc| !doc.saving && doc.external.is_none()),
            "save-as" | "save-copy" => {
                document.is_some_and(|doc| !doc.saving) && !self.destination_busy
            }
            "find" | "line" | "close" => document.is_some(),
            "read" => document.is_some_and(Document::markdown),
            "keep-tab" => document.is_some_and(|doc| doc.preview),
            "indent" | "outdent" | "paste" | "select-all" => document.is_some() && !self.reading(),
            "undo" => document.is_some() && !self.reading() && self.edit_state.get()[0],
            "redo" => document.is_some() && !self.reading() && self.edit_state.get()[1],
            "cut" | "copy" => document.is_some() && !self.reading() && self.edit_state.get()[2],
            "reload" | "reveal" => {
                document.is_some_and(|doc| !doc.disk.target.as_os_str().is_empty() && !doc.saving)
            }
            "compare" | "overwrite" => {
                document.is_some_and(|doc| doc.external.is_some() && !doc.saving)
            }
            "rename" | "trash" => self
                .selection
                .as_ref()
                .is_some_and(|path| self.file_action_blocker(path, command == "trash").is_none()),
            "stop-search" => self.search_ui.running,
            "remove-index" => self.search.index_status(self.effective_scope()).is_some(),
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
        let enabled = self.command_enabled(command);
        let dirty_save = command == "save"
            && enabled
            && self
                .documents
                .iter()
                .any(|doc| Some(doc.id) == self.active && doc.dirty(cx));
        Button::new(id)
            .label(label)
            .small()
            .compact()
            .disabled(!enabled)
            .map(|button| if dirty_save { button.primary() } else { button })
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }
    /// F6 moves between the main areas, as in other desktop applications, so the
    /// keyboard can leave the source editor where Tab inserts indentation.
    fn cycle_focus(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let narrow = self.narrow(window);
        let mut areas: Vec<u8> = Vec::new();
        if self.panel.is_some() {
            areas.push(0);
        }
        if self.navigator_visible && (!narrow || self.navigation_only || self.active.is_none()) {
            areas.push(1);
        }
        if self.active.is_some() && (!narrow || !self.navigation_only) {
            if self
                .documents
                .iter()
                .any(|doc| Some(doc.id) == self.active && doc.external.is_some())
            {
                areas.push(3);
            }
            areas.push(2);
        }
        if let Some(failure) = &self.open_failure {
            let anchor = match &failure.return_focus {
                Some(focus) if focus == &self.nav_focus => 1,
                Some(focus)
                    if focus == &self.search_ui.focus || focus == &self.query.focus_handle(cx) =>
                {
                    0
                }
                _ => 2,
            };
            let position = areas
                .iter()
                .position(|&area| area == anchor)
                .map_or(areas.len(), |index| index + 1);
            areas.insert(position, 4);
        }
        if areas.is_empty() {
            return;
        }
        let current = if self.notice_focus.contains_focused(window, cx) {
            Some(4)
        } else if self.query.focus_handle(cx).contains_focused(window, cx)
            || self.search_ui.focus.is_focused(window)
        {
            Some(0)
        } else if self.nav_focus.is_focused(window) {
            Some(1)
        } else if self.conflict_focus.is_focused(window) {
            Some(3)
        } else if self.reading_focus.contains_focused(window, cx)
            || self.documents.iter().any(|doc| {
                doc.input.focus_handle(cx).contains_focused(window, cx)
                    || doc.tab_focus.contains_focused(window, cx)
            })
        {
            Some(2)
        } else {
            None
        };
        let position = current.and_then(|area| areas.iter().position(|a| *a == area));
        let next = match position {
            Some(index) if reverse => areas[(index + areas.len() - 1) % areas.len()],
            Some(index) => areas[(index + 1) % areas.len()],
            None => areas[0],
        };
        match next {
            0 => self.query.update(cx, |input, cx| input.focus(window, cx)),
            1 => window.focus(&self.nav_focus),
            3 => window.focus(&self.conflict_focus),
            4 => window.focus(&self.notice_details_focus),
            _ => {
                if self.reading() {
                    self.reading_focus.focus(window);
                } else {
                    self.focus(window, cx);
                }
            }
        }
        cx.notify();
    }
    /// How many optional toolbar groups fit, measured from the labels at the current
    /// scale: 0 shows everything; higher tiers move Commands, then Filter and Find Text,
    /// then Open, then Back/Forward/Parent into the menu, and finally show Find Files,
    /// the view switch and Locations as icons.
    fn toolbar_tier(&self, narrow: bool, window: &Window) -> usize {
        let rem = window.rem_size();
        let gap = px(4. * self.settings.ui_scale);
        let text = |label: &str| {
            let label = SharedString::from(label.to_owned());
            let run = window.text_style().to_run(label.len());
            window
                .text_system()
                .shape_line(label, rem * 0.875, &[run], None)
                .width
                + rem * 1.75
        };
        let icon = rem * 1.5;
        let with_icon = |label: &str| text(label) + rem + gap;
        let toggle = if narrow {
            with_icon("Navigator").max(text("Open…"))
        } else {
            icon
        };
        let mut required =
            rem * 2. + toggle + gap * 2. + with_icon("Locations") + with_icon("Find Files") + icon;
        if self.request.pick {
            required += text("Choose") + text("Cancel") + gap * 2.;
        }
        let optional = [
            text("Commands") + gap * 2.,
            text("Filter This Folder") + text("Find Text") + gap * 2.,
            text("Open…") + gap,
            icon * 3. + gap * 4.,
            with_icon("Find Files") - icon,
            if narrow { toggle - icon } else { px(0.) },
            with_icon("Locations") - icon,
        ];
        let width = window.viewport_size().width;
        let mut total = required + optional.iter().copied().fold(px(0.), |sum, w| sum + w);
        for (tier, item) in optional.iter().enumerate() {
            if total <= width {
                return tier;
            }
            total -= *item;
        }
        optional.len()
    }
    fn tool_button(
        &self,
        command: &'static str,
        label: &'static str,
        icon: Option<IconName>,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(SharedString::from(format!("tool-{command}")))
            .label(label)
            .when_some(icon, |button, icon| button.icon(icon))
            .ghost()
            .when(self.command_enabled(command), |button| {
                button.text_color(cx.theme().muted_foreground)
            })
            .small()
            .disabled(!self.command_enabled(command))
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }
    fn icon_button(
        &self,
        command: &'static str,
        label: &'static str,
        icon: IconName,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(command)
            .icon(icon)
            .tooltip(label)
            .ghost()
            .when(self.command_enabled(command), |button| {
                button.text_color(cx.theme().muted_foreground)
            })
            .small()
            .disabled(!self.command_enabled(command))
            .on_click(cx.listener(move |this, _, window, cx| this.command(command, window, cx)))
    }
    fn select_tab(&mut self, id: u64, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.documents.iter_mut().find(|doc| doc.id == id) else {
            return;
        };
        if keep {
            document.preview = false;
        }
        self.clear_open_failure();
        self.active = Some(id);
        self.navigation_only = false;
        self.focus(window, cx);
        cx.notify();
    }
}
impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.edit_state.set(
            self.documents
                .iter()
                .find(|doc| Some(doc.id) == self.active)
                .map(|doc| {
                    let input = doc.input.read(cx);
                    [input.can_undo(), input.can_redo(), input.has_selection()]
                })
                .unwrap_or_default(),
        );
        if self.reading()
            && !self
                .documents
                .iter()
                .any(|doc| Some(doc.id) == self.active && doc.markdown())
        {
            self.set_reading(false);
        }
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
        let source_visible = !self.reading() && (!narrow || !self.navigation_only);
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
        let scale = self.settings.ui_scale;
        let main_menu_requested = std::mem::take(&mut self.main_menu_requested);
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap(px(4. * scale))
            .px(px(8. * scale))
            .py(px(6. * scale))
            .when(short, |toolbar| toolbar.py(px(3. * scale)))
            .bg(cx.theme().title_bar)
            .when(compact_document, |toolbar| {
                toolbar
                    .border_b_1()
                    .border_color(cx.theme().title_bar_border)
            });
        let tier = self.toolbar_tier(narrow, window);
        toolbar = if narrow && self.active.is_none() && !self.request.pick {
            toolbar.child(self.tool_button("open", "Open…", None, cx))
        } else if narrow {
            let (label, icon) = if self.navigation_only {
                ("Document", IconName::File)
            } else {
                ("Navigator", IconName::Folder)
            };
            if tier >= 6 {
                toolbar.child(self.icon_button("toggle-navigator", label, icon, cx))
            } else {
                toolbar.child(self.tool_button("toggle-navigator", label, Some(icon), cx))
            }
        } else {
            toolbar.child(self.icon_button(
                "toggle-navigator",
                if self.navigator_visible {
                    "Hide Navigator"
                } else {
                    "Show Navigator"
                },
                if self.navigator_visible {
                    IconName::PanelLeftClose
                } else {
                    IconName::PanelLeftOpen
                },
                cx,
            ))
        };
        toolbar = toolbar
            .when(tier <= 3, |toolbar| {
                toolbar
                    .child(div().w(px(4. * scale)))
                    .child(self.icon_button("back", "Back", IconName::ArrowLeft, cx))
                    .child(self.icon_button("forward", "Forward", IconName::ArrowRight, cx))
                    .child(self.icon_button("parent", "Parent Folder", IconName::ArrowUp, cx))
            })
            .child(div().w(px(4. * scale)))
            .child(self.locations_menu(tier >= 7, cx))
            .when(tier <= 2 && !(narrow && self.active.is_none()), |toolbar| {
                toolbar.child(self.tool_button("open", "Open…", None, cx))
            })
            .child(div().flex_1())
            .when(tier <= 1, |toolbar| {
                toolbar.child(self.tool_button("filter", "Filter This Folder", None, cx))
            })
            .child(if tier >= 5 {
                self.icon_button("find-files", "Find Files", IconName::Search, cx)
            } else {
                self.tool_button("find-files", "Find Files", None, cx)
            })
            .when(tier <= 1, |toolbar| {
                toolbar.child(self.tool_button("find-text", "Find Text", None, cx))
            })
            .when(tier == 0, |toolbar| {
                toolbar
                    .child(div().w(px(4. * scale)))
                    .child(self.tool_button("commands", "Commands", None, cx))
            })
            .child(self.main_menu(main_menu_requested, cx));
        if self.request.pick {
            toolbar = toolbar
                .child(self.button("Choose", "choose", cx))
                .child(self.button("Cancel", "cancel", cx));
        }
        let root = div()
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
            .relative()
            .when(!compact_document, |root| {
                root.child(self.path_bar(short, window, cx))
            })
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
        let takeover = self.panel_takes_over(narrow, window);
        self.search_ui.takeover = takeover;
        if self.search_ui.focus.is_focused(window)
            && self.search_ui.selection.is_none()
            && !self.hits.is_empty()
        {
            self.search_ui.selection = Some(0);
            self.search_ui.picker_selection = Some(self.hits[0].path.clone());
        }
        let mut main = div().flex_1().min_h_0().overflow_hidden().flex().flex_col();
        // Commands and path completions float above the panes so typing never moves
        // the document; search panels keep a fixed height while open.
        let mut floating = None;
        if self.panel == Some(Panel::Path) {
            if takeover {
                main = main.child(self.path_matches(window, cx));
            } else {
                floating = Some(self.path_matches(window, cx));
            }
        }
        if let Some(panel) = self.panel.filter(|panel| *panel != Panel::Path) {
            self.palette.floating = panel == Panel::Palette && !takeover;
            if self.palette.floating {
                floating = Some(self.palette_overlay(window, cx));
            } else if panel == Panel::Filter {
                // Filtering belongs to the navigator; it opens there and leaves the document still.
            } else {
                main = main.child(self.panel_view(panel, takeover, window, cx));
            }
        }
        if self.finishing.pending() {
            main = main.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .flex_shrink_0()
                    .child("Validating requested destinations…")
                    .child(self.button("Cancel completion", "cancel-close", cx)),
            );
        }
        if !self.closings.is_empty() {
            main = main.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .flex_shrink_0()
                    .child("Resolving recovery before completing the decision")
                    .child(self.button("Retry cleanup", "retry-close", cx))
                    .child(self.button("Keep open", "cancel-close", cx)),
            );
        }

        let visible = self.visible_rows();
        let entries = self.entries.clone();
        // A focused list always has a current row, so screen readers announce a file rather
        // than only the list.
        if self.nav_focus.is_focused(window)
            && self.listing_done
            && self.reveal_pending.is_none()
            && self
                .trash_pending
                .as_ref()
                .is_none_or(|request| Some(&request.path) != self.selection.as_ref())
            && !visible
                .iter()
                .any(|&i| Some(&entries[i].path) == self.selection.as_ref())
            && let Some(&first) = visible.first()
        {
            self.selection = Some(entries[first].path.clone());
            self.search_ui.picker_selection = self.selection.clone();
        }
        let scroll = self
            .scrolls
            .entry(self.current.clone())
            .or_default()
            .clone();
        if self.listing_done
            && let Some(path) = self.reveal_pending.as_ref()
            && let Some(index) = visible
                .iter()
                .position(|&index| &entries[index].path == path)
        {
            self.reveal_pending = None;
            scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
        let nav_width = if narrow {
            window.viewport_size().width
        } else {
            self.pane_state.read(cx).width(window)
        };
        let navigator = div()
            .id("navigator")
            .track_focus(&self.nav_focus)
            .w_full()
            .h_full()
            .bg(cx.theme().sidebar)
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
                let failed = self.listing_error.is_some();
                let scale = self.settings.ui_scale;
                navigator.child(
                    div().m(px(6. * scale)).px(px(10. * scale)).py(px(8. * scale)).text_sm()
                        .rounded(px(7. * scale))
                        .when(failed, |notice| notice.bg(cx.theme().warning.opacity(0.1)).border_l(px(3. * scale)).border_color(cx.theme().warning))
                        .when(!failed, |notice| notice.text_color(cx.theme().muted_foreground))
                        .child(accessibility::label("listing-state-text", message.clone(), div().id("listing-state").whitespace_normal().line_clamp(4).child(message.clone())
                            .tooltip(move |window,cx| gpui_component::tooltip::Tooltip::new(message.clone()).build(window,cx))))
                        .child(
                            div().flex().flex_wrap().gap_1().mt_1()
                                .child(self.button("Refresh", "refresh", cx))
                                .when(!self.listing_done, |row| row.child(self.button("Stop Reading", "stop-listing", cx)))
                                .when(self.listing_error.is_some(), |row| row.child(self.button("Details…", "folder-error", cx))),
                        )
                )
            })
            .when(!visible.is_empty(), |navigator| navigator.child(
                uniform_list(
                    "entries",
                    visible.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                        let scale = this.settings.ui_scale;
                        let focused = this.nav_focus.is_focused(window);
                        let name_width = nav_width - px((12. + 3. + 9. + 12. + 16. + 8. + 4.) * scale);
                        let font_size = px(14. * scale);
                        let measure = |text: &SharedString| {
                            let run = window.text_style().to_run(text.len());
                            window
                                .text_system()
                                .shape_line(text.clone(), font_size, &[run], None)
                                .width
                        };
                        range
                            .map(|i| {
                                let entry = &entries[visible[i]];
                                let path = entry.path.clone();
                                let cursor = this.selection.as_ref() == Some(&path);
                                let selected = cursor && !this.trash_needs_selection;
                                let cursor_border = if cursor && (focused || this.navigator_menu.is_some()) && !selected {
                                    cx.theme().ring
                                } else {
                                    gpui::transparent_black()
                                };
                                let tooltip = paths::display(&path);
                                let name = paths::name(&path);
                                let caption = labels::filename(
                                    &name, "", this.navigation_hints.get(&name), name_width, &measure,
                                );
                                let icon = if entry.symlink {
                                    IconName::ExternalLink
                                } else if entry.directory {
                                    IconName::Folder
                                } else {
                                    IconName::File
                                };
                                let (marker, fill) = theme::list_row(selected, focused, cx);
                                let context_path = path.clone();
                                let row=div()
                                    .id(("entry", i))
                                    .h_full()
                                    .w_full()
                                    .rounded(px(6. * scale))
                                    .flex()
                                    .items_center()
                                    .gap(px(8. * scale))
                                    .border_l(px(3. * scale))
                                    .border_y_1()
                                    .border_r_1()
                                    .border_color(if selected { marker } else { cursor_border })
                                    .when(selected, |row| row.border_y_0().border_r_0())
                                    .pl(px(9. * scale))
                                    .pr(px(12. * scale))
                                    .bg(fill.unwrap_or(cx.theme().sidebar))
                                    .hover(|row| if selected { row } else { row.bg(cx.theme().list_hover) })
                                    .when(this.navigator_menu.is_none(), |row| {
                                        row.tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx))
                                    })
                                    .cursor_pointer()
                                    .child(
                                        gpui_component::Icon::new(icon)
                                            .with_size(px(16. * scale))
                                            .flex_shrink_0()
                                            .text_color(if entry.directory && !entry.symlink {
                                                cx.theme().ring
                                            } else {
                                                cx.theme().muted_foreground
                                            }),
                                    )
                                    .child(div().min_w_0().flex_1().truncate().child(caption))
                                    .on_mouse_down(MouseButton::Right, cx.listener(
                                        move |this, event: &MouseDownEvent, window, cx| {
                                            this.navigator_menu_from_pointer(context_path.clone(), event.position, window, cx);
                                            cx.stop_propagation();
                                        },
                                    ))
                                    .on_click(cx.listener(
                                        move |this, event: &ClickEvent, window, cx| {
                                            this.select_navigation(path.clone());
                                            window.focus(&this.nav_focus);
                                            this.open(path.clone(), event.click_count() < 2, None);
                                            cx.notify();
                                        },
                                    ));
                                let row = div()
                                    .h(this.navigation_row_height())
                                    .w_full()
                                    .px(px(6. * scale))
                                    .py(px(1. * scale))
                                    .child(row);
                                this.navigation_row(entry,(i,visible.len()),cursor,row,cx)
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
        let focused_tab = self
            .documents
            .iter()
            .position(|doc| doc.tab_focus.contains_focused(window, cx));
        let shown_tab = focused_tab.or_else(|| {
            self.documents
                .iter()
                .position(|doc| Some(doc.id) == self.active)
        });
        if let Some(index) = shown_tab {
            let states = self
                .documents
                .iter()
                .map(|doc| (doc.id, doc.dirty(cx), doc.preview, doc.external.is_some()))
                .collect();
            let reveal = (index, window.viewport_size(), states);
            if self.tab_reveal.as_ref() != Some(&reveal) {
                self.tab_reveal = Some(reveal);
                self.tab_scroll.scroll_to_item(index);
            }
        }
        let edit_source = compact_document && self.reading();
        let show_documents_menu = self.documents.len() > 1 && !self.request.pick;
        let tab_conflict = short
            && self
                .documents
                .iter()
                .any(|doc| Some(doc.id) == self.active && doc.external.is_some());
        let rem = window.rem_size();
        let documents_full =
            Self::path_label_width(window, &"Documents".into()) + rem * 3. + px(2.);
        let edit_full = Self::path_label_width(window, &"Edit Source".into()) + rem * 2.;
        let close_width = if self.request.pick { px(0.) } else { rem * 1.5 };
        let conflict_width = if tab_conflict {
            rem * 1.5 + px(10. * scale)
        } else {
            px(0.)
        };
        let active_markdown = self
            .documents
            .iter()
            .any(|doc| Some(doc.id) == self.active && doc.markdown());
        let document_actions = !self.request.pick && !short && self.active.is_some();
        let read_label = if self.reading() {
            "Edit Source"
        } else {
            "Read"
        };
        let keep_tab = self.command_enabled("keep-tab");
        let actions_width = if document_actions {
            let gap = px(8. * scale);
            let button = |label: &str| {
                Self::path_label_width(window, &SharedString::from(label.to_owned()))
                    + rem * 0.75
                    + px(2.)
                    + gap
            };
            button("Save")
                + if active_markdown {
                    button(read_label)
                } else {
                    px(0.)
                }
                + if keep_tab {
                    button("Keep Preview Tab")
                } else {
                    px(0.)
                }
                + rem * 1.5
                + gap
        } else {
            px(0.)
        };
        let controls_width = |documents: Pixels, edit: Pixels| {
            actions_width
                + (if show_documents_menu {
                    documents + px(8. * scale)
                } else {
                    px(0.)
                })
                + (if edit_source {
                    edit + px(8. * scale)
                } else {
                    px(0.)
                })
                + conflict_width
                + px(10. * scale)
        };
        // Keep room for a recognizable active tab: shorten the controls beside the tabs
        // before the tab itself.
        let compact_tab_controls = narrow
            && window.viewport_size().width
                - controls_width(documents_full, edit_full)
                - close_width
                < rem * 10.;
        let tab_controls_width = if compact_tab_controls {
            controls_width(
                rem * 2.,
                Self::path_label_width(window, &"Edit".into()) + rem * 2.,
            )
        } else {
            controls_width(documents_full, edit_full)
        };
        let navigator_width = if !narrow && self.navigator_visible {
            nav_width + px(8.)
        } else {
            px(0.)
        };
        let tab_width =
            (window.viewport_size().width - navigator_width - tab_controls_width - close_width)
                .max(rem * 1.5);
        let tab_parents =
            paths::parent_hints(self.documents.iter().map(|doc| doc.disk.path.as_path()));
        let tab_filenames = paths::filename_hints(
            self.documents
                .iter()
                .filter_map(|doc| doc.disk.path.file_name()),
        );
        // Like a scrolling notebook, the strip shows the current tab and as many neighbours as
        // fit whole, from widths measured in the last frame, so no fragment of a tab is left at
        // an edge. Hidden tabs stay reachable with Ctrl+Tab, the arrow keys and Documents.
        for (index, doc) in self.documents.iter().enumerate() {
            if let Some(bounds) = self.tab_scroll.bounds_for_item(index)
                && bounds.size.width > px(0.)
                && self.tab_widths.insert(doc.id, bounds.size.width) != Some(bounds.size.width)
            {
                cx.notify();
            }
        }
        self.tab_widths
            .retain(|id, _| self.documents.iter().any(|doc| doc.id == *id));
        let room = self.tab_scroll.bounds().size.width - px(8. * scale);
        let shown_tabs = shown_tab.filter(|_| room > px(0.)).map(|center| {
            let width = |index: usize| {
                self.tab_widths
                    .get(&self.documents[index].id)
                    .copied()
                    .unwrap_or_default()
            };
            let gap = px(2. * scale);
            let (mut first, mut last, mut used) = (center, center, width(center));
            loop {
                let mut grew = false;
                if last + 1 < self.documents.len() && used + gap + width(last + 1) <= room {
                    last += 1;
                    used += gap + width(last);
                    grew = true;
                }
                if first > 0 && used + gap + width(first - 1) <= room {
                    first -= 1;
                    used += gap + width(first);
                    grew = true;
                }
                if !grew {
                    break first..=last;
                }
            }
        });
        let document_focused = self.reading_focus.contains_focused(window, cx)
            || self.documents.iter().any(|doc| {
                doc.input.focus_handle(cx).contains_focused(window, cx)
                    || doc.tab_focus.contains_focused(window, cx)
            });
        let tabs = div()
            .id("tabs")
            .w_full()
            .flex()
            .items_end()
            .gap(px(2. * scale))
            .px(px(4. * scale))
            .pt(px(4. * scale))
            .when(short, |tabs| tabs.pt_0())
            .overflow_x_scroll()
            .flex_shrink_0()
            .track_scroll(&self.tab_scroll)
            .children(self.documents.iter().enumerate().map(|(index, doc)| {
                let id = doc.id;
                let hidden = shown_tabs
                    .as_ref()
                    .is_some_and(|shown| !shown.contains(&index));
                let name = paths::tab_label(
                    &doc.disk.path,
                    self.documents.iter().map(|doc| doc.disk.path.as_path()),
                );
                let dirty = doc.dirty(cx);
                let active = Some(id) == self.active;
                let suffix = match (dirty, doc.external.is_some() && !(active && tab_conflict)) {
                    (false, false) => "",
                    (true, false) => " •",
                    (false, true) => " (conflict)",
                    (true, true) => " • (conflict)",
                };
                let filename = paths::name(&doc.disk.path);
                let measure = |text: &SharedString| Self::path_label_width(window, text);
                let text_width = (tab_width - rem * 0.75 - px(2.)).max(px(0.));
                let caption = match tab_parents.get(&doc.disk.path) {
                    Some(folder) => {
                        labels::document(&filename, suffix, folder, " — ", text_width, &measure)
                    }
                    None => labels::filename(
                        &filename,
                        suffix,
                        tab_filenames.get(&filename),
                        text_width,
                        &measure,
                    ),
                };
                let spoken = [
                    Some(name.as_str()),
                    dirty.then_some("unsaved changes"),
                    doc.preview.then_some("preview"),
                    doc.external.is_some().then_some("changed on disk"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
                let tab = Button::new(("tab", id))
                    .with_focus_handle(doc.tab_focus.clone())
                    .tab_stop(active)
                    .accessibility_role(gpui::accesskit::Role::Tab)
                    .label(spoken)
                    .max_w(tab_width)
                    .display_label(caption)
                    .tooltip(paths::display(&doc.disk.path))
                    .ghost()
                    .small()
                    .compact()
                    .text_color(if active {
                        cx.theme().tab_active_foreground
                    } else {
                        cx.theme().tab_foreground
                    })
                    .when(active, |tab| tab.bg(cx.theme().tab_active))
                    .selected(active)
                    .when(doc.preview, |tab| tab.italic_label())
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.select_tab(id, event.click_count() > 1, window, cx);
                    }));
                div()
                    .flex()
                    .when(hidden, |tab| tab.hidden())
                    .items_center()
                    .flex_shrink_0()
                    .h_full()
                    .px(px(2. * scale))
                    .rounded_t(px(7. * scale))
                    .border_t_2()
                    .border_color(match (active, document_focused) {
                        (true, true) => cx.theme().ring,
                        (true, false) => cx.theme().muted_foreground,
                        _ => gpui::transparent_black(),
                    })
                    .when(active, |tab| tab.bg(cx.theme().tab_active))
                    .child(tab)
                    .when(!self.request.pick, |tab| {
                        tab.child(
                            Button::new(("close-tab", id))
                                .icon(IconName::Close)
                                .tab_stop(false)
                                .small()
                                .ghost()
                                .text_color(if active {
                                    cx.theme().tab_active_foreground
                                } else {
                                    cx.theme().tab_foreground
                                })
                                .when(active, |close| close.bg(cx.theme().tab_active))
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
        let status = if self.status == "Saved"
            && !self.documents.iter().any(|doc| {
                self.active == Some(doc.id) && self.last_saved == Some((doc.id, doc.generation))
            }) {
            ""
        } else {
            self.status.as_str()
        };
        let has_status = !status.is_empty() || self.font_warning.is_some();
        let footer_height = if narrow { 28. } else { 36. };
        let footer_height = px(footer_height * self.settings.ui_scale);
        let open_documents_requested = std::mem::take(&mut self.open_documents_requested);
        let tabs = div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .when(self.documents.is_empty(), |row| row.hidden())
            .gap(px(8. * scale))
            .bg(cx.theme().tab_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .when(
                show_documents_menu || edit_source || tab_conflict || document_actions,
                |row| row.pr(px(8. * scale)),
            )
            .child(div().flex_1().min_w_0().child(tabs))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(8. * scale))
                    // Centre the controls on the tab labels, which sit below a 2-point top edge.
                    .pt(px(if short { 2. } else { 6. } * scale))
                    .when(tab_conflict, |row| {
                        row.child(accessibility::message(
                            "external-conflict",
                            "Changed on disk. Local text retained; ordinary Save is blocked."
                                .into(),
                            div().size_0(),
                        ))
                        .child(self.conflict_menu(true, cx))
                    })
                    .when(show_documents_menu, |row| {
                        row.child(self.open_documents_menu(
                            open_documents_requested,
                            footer_height,
                            compact_tab_controls,
                            &tab_parents,
                            &tab_filenames,
                            cx,
                        ))
                    })
                    .when(document_actions, |row| {
                        row.child(
                            div()
                                .flex()
                                .items_center()
                                .flex_shrink_0()
                                .gap(px(8. * scale))
                                .child(self.button("Save", "save", cx))
                                .when(active_markdown, |row| {
                                    row.child(self.button(read_label, "read", cx))
                                })
                                .when(keep_tab, |row| {
                                    row.child(self.button("Keep Preview Tab", "keep-tab", cx))
                                }),
                        )
                    })
                    .when(edit_source, |row| {
                        row.child(
                            self.button("Edit Source", "read", cx)
                                .when(compact_tab_controls, |button| button.display_label("Edit"))
                                .flex_shrink_0(),
                        )
                    })
                    .when(document_actions, |row| row.child(self.document_menu(cx))),
            )
            .into_any_element();
        let mut content = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs);
        // Scope membership matters while searching or with a pinned scope, not while browsing.
        let outside_scope = (matches!(self.panel, Some(Panel::Files | Panel::Text))
            || self.scope.is_some()
            || self.search_ui.temporary.is_some())
            && self.documents.iter().any(|doc| {
                Some(doc.id) == self.active
                    && !doc.disk.target.as_os_str().is_empty()
                    && !doc.disk.path.starts_with(self.effective_scope())
            });
        if document_actions && outside_scope {
            content = content.child(
                div()
                    .flex()
                    .justify_end()
                    .flex_shrink_0()
                    .px(px(8. * scale))
                    .pt(px(4. * scale))
                    .child(
                        Button::new("document-search-scope")
                            .label(format!(
                                "Outside search folder: {}",
                                paths::name(self.effective_scope())
                            ))
                            .tooltip(paths::display(self.effective_scope()))
                            .ghost()
                            .small()
                            .min_w_0()
                            .flex_shrink()
                            .text_color(cx.theme().muted_foreground)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.command("search-details", window, cx)
                            })),
                    ),
            );
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            let mut notices = div()
                .id("document-notices")
                .flex()
                .flex_col()
                .flex_shrink_0()
                .max_h(relative(0.5))
                .overflow_y_scroll();
            let mut has_notices = false;
            if doc.recovery_error.is_none()
                && let Some(error) = &doc.error
            {
                has_notices = true;
                let title = match doc.failure_action {
                    FailureAction::Save => "Last save failed",
                    FailureAction::Reload => "Reload stopped",
                    FailureAction::Check => "File unavailable",
                    FailureAction::Choose(_) => "Destination unavailable",
                };
                notices = notices.child(
                    self.notice(cx)
                        .child(self.notice_label(
                            "operation-failure",
                            title.into(),
                            error,
                            true,
                            cx,
                        ))
                        .child(self.button("Details and Retry…", "failure-details", cx)),
                );
            }
            if let Some((_, error)) = &doc.recovery_error {
                has_notices = true;
                notices = notices.child(self.recovery_failure_view(doc, error, cx));
            }
            let compact_notice =
                narrow || window.viewport_size().height < px(560. * self.settings.ui_scale);
            let conflict_reason = "Local text retained; ordinary Save is blocked.";
            if doc.external.is_some() && compact_notice && !short {
                has_notices = true;
                notices = notices.child(
                    self.notice(cx)
                        .child(self.notice_label(
                            "external-conflict",
                            "Changed on disk".into(),
                            conflict_reason,
                            false,
                            cx,
                        ))
                        .child(self.conflict_menu(false, cx)),
                );
            } else if doc.external.is_some() && !compact_notice {
                has_notices = true;
                notices = notices.child(
                    self.notice(cx)
                        .child(self.notice_label(
                            "external-conflict",
                            "Changed on disk".into(),
                            conflict_reason,
                            true,
                            cx,
                        ))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    self.button("Compare", "compare", cx)
                                        .with_focus_handle(self.conflict_focus.clone()),
                                )
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
            if has_notices {
                content = content.child(accessibility::group(
                    "document-notices",
                    "Document notices".into(),
                    notices,
                ));
            }
            if self.reading() {
                content = content.child(self.reading_element(narrow, short, cx));
            } else {
                content = content.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .when(short, |editor| editor.pt(px(4. * scale)))
                        .font_family(if self.settings.proportional && doc.grammar(cx).is_none() {
                            cx.theme().font_family.clone()
                        } else {
                            cx.theme().mono_font_family.clone()
                        })
                        .child(
                            Input::new(&doc.input)
                                .bordered(false)
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
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12. * scale))
                    .p(px(16. * scale))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child("Select a file to edit"),
                    )
                    .when(!self.request.pick, |empty| {
                        empty.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .justify_center()
                                .gap(px(8. * scale))
                                .child(self.button("Open File…", "open", cx))
                                .child(self.button("New File", "new", cx)),
                        )
                    }),
            );
        }
        let body = if narrow {
            if self.navigation_only || self.active.is_none() {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .when(self.panel == Some(Panel::Filter), |pane| {
                        pane.child(self.navigator_filter(cx))
                    })
                    .child(div().flex_1().min_h_0().child(navigator))
                    .into_any_element()
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
                        .flex()
                        .flex_col()
                        .when(self.panel == Some(Panel::Filter), |pane| {
                            pane.child(self.navigator_filter(cx))
                        })
                        .child(div().flex_1().min_h_0().child(navigator)),
                )
                .child(self.pane_state.clone())
                .child(content)
                .into_any_element()
        } else {
            content.into_any_element()
        };
        root.child(
            main.relative()
                .when(!takeover, |main| {
                    main.child(div().flex_1().min_h_0().overflow_hidden().child(body))
                })
                .children(floating)
                .when(self.open_failure.is_some(), |main| {
                    main.child(self.open_failure_overlay(window, cx))
                })
                .when_some(
                    self.query_error
                        .clone()
                        .filter(|_| self.panel == Some(Panel::Path)),
                    |main, error| main.child(self.path_error_view(error, cx)),
                ),
        )
        .child(
            div()
                .h(footer_height)
                .flex_shrink_0()
                .px_2()
                .bg(cx.theme().title_bar)
                .border_t_1()
                .border_color(cx.theme().title_bar_border)
                .text_color(cx.theme().muted_foreground)
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12. * self.settings.ui_scale))
                .when(!self.warnings.is_empty(), |footer| {
                    footer.child(self.button("Warnings…", "warnings", cx))
                })
                .when(
                    self.warnings.is_empty() && (!narrow || has_status),
                    |footer| {
                        let message = match &self.font_warning {
                            Some(warning) if status.is_empty() => warning.clone(),
                            Some(warning) => format!("{status} · {warning}"),
                            None => status.to_owned(),
                        };
                        let status = div()
                            .id("status-message")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .map(|status| {
                                if let Some((reason, target)) = message.split_once('\n') {
                                    let line = |text: &str| {
                                        div()
                                            .truncate()
                                            .line_height(px(13.5 * scale))
                                            .child(text.to_owned())
                                    };
                                    status.child(line(reason)).child(line(target))
                                } else {
                                    status.child(message.clone())
                                }
                            });
                        let tooltip = message.clone();
                        footer.child(accessibility::message(
                            "application-status",
                            message,
                            status.tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(tooltip.clone())
                                    .build(window, cx)
                            }),
                        ))
                    },
                )
                .when(
                    narrow && (!has_status || !self.warnings.is_empty()),
                    |footer| footer.child(div().flex_1()),
                )
                .when_some(
                    (!narrow || !self.navigation_only)
                        .then(|| {
                            self.documents
                                .iter()
                                .find(|doc| Some(doc.id) == self.active)
                        })
                        .flatten(),
                    |footer, doc| {
                        let information = doc.information(cx, self.reading());
                        footer.child(accessible(
                            "document-information",
                            div()
                                .truncate()
                                .min_w_0()
                                .when(has_status || !self.warnings.is_empty(), |info| {
                                    info.max_w(relative(0.4))
                                })
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
        .when(
            compact_document && self.panel == Some(Panel::Path),
            |root| {
                root.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .occlude()
                        .bg(cx.theme().background)
                        .child(self.path_bar(short, window, cx)),
                )
            },
        )
        .children(self.navigator_menu_view())
        .children(self.preferences.clone())
        .children(self.decision.clone())
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if this.preferences.is_some() || this.decision.is_some() {
                return;
            }
            let key = &event.keystroke;
            if this.palette.focus.is_focused(window) && this.palette_key(key, window, cx) {
                return;
            }
            if this.search_ui.focus.is_focused(window) && this.search_key(key, window, cx) {
                return;
            }
            if let Some(index) = this
                .documents
                .iter()
                .position(|doc| doc.tab_focus.is_focused(window))
                && !key.modifiers.control
                && !key.modifiers.platform
                && !key.modifiers.alt
            {
                let last = this.documents.len() - 1;
                let next = match key.key.as_str() {
                    "left" => Some(index.checked_sub(1).unwrap_or(last)),
                    "right" => Some(if index == last { 0 } else { index + 1 }),
                    "home" => Some(0),
                    "end" => Some(last),
                    _ => None,
                };
                if let Some(next) = next {
                    let id = this.documents[next].id;
                    this.clear_open_failure();
                    this.active = Some(id);
                    this.navigation_only = false;
                    window.focus(&this.documents[next].tab_focus.clone());
                    cx.notify();
                    return;
                }
            }
            if this.nav_focus.is_focused(window)
                && !key.modifiers.control
                && !key.modifiers.platform
            {
                if key.modifiers.alt {
                    return;
                }
                let visible = this.visible_rows();
                let rows = this.entries.clone();
                let index = visible
                    .iter()
                    .position(|&i| Some(&rows[i].path) == this.selection.as_ref());
                match key.key.as_str() {
                    "f2" | "delete" if !key.modifiers.shift => {
                        let command = if key.key == "f2" { "rename" } else { "trash" };
                        if !event.is_held && !this.request.pick {
                            this.command(command, window, cx);
                        }
                        return;
                    }
                    "space" if !key.modifiers.shift && this.trash_needs_selection => {
                        if let Some(path) = this.selected_path() {
                            this.select_navigation(path);
                            cx.notify();
                        }
                        return;
                    }
                    "f10" if key.modifiers.shift => {
                        this.navigator_menu_from_keyboard(&visible, index, window, cx);
                        return;
                    }
                    "menu" if !key.modifiers.shift => {
                        this.navigator_menu_from_keyboard(&visible, index, window, cx);
                        return;
                    }
                    "up" | "down" | "home" | "end" | "pageup" | "pagedown"
                        if !visible.is_empty() && !key.modifiers.shift =>
                    {
                        let Some(next) = keyboard::list_step(&key.key, index, visible.len(), 10)
                        else {
                            return;
                        };
                        this.select_navigation(rows[visible[next]].path.clone());
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
                // Type-ahead: printable keys select the first row whose name starts with
                // the letters typed in quick succession.
                if let Some(typed) = key
                    .key_char
                    .as_deref()
                    .filter(|text| !key.modifiers.alt && text.chars().all(|c| !c.is_control()))
                    .filter(|text| !text.is_empty() && *text != " ")
                {
                    let now = std::time::Instant::now();
                    if this
                        .type_ahead
                        .1
                        .is_none_or(|last| now - last > std::time::Duration::from_millis(900))
                    {
                        this.type_ahead.0.clear();
                    }
                    this.type_ahead.0.push_str(&typed.to_lowercase());
                    this.type_ahead.1 = Some(now);
                    let prefix = this.type_ahead.0.clone();
                    if let Some(next) = visible.iter().position(|&i| {
                        paths::name(&rows[i].path)
                            .to_lowercase()
                            .starts_with(&prefix)
                    }) {
                        this.select_navigation(rows[visible[next]].path.clone());
                        if let Some(scroll) = this.scrolls.get(&this.current) {
                            scroll.scroll_to_item(next, ScrollStrategy::Center);
                        }
                        cx.notify();
                    }
                    return;
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
