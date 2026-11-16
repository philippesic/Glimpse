//! Each picker is a separate, read-only request with one explicitly validated result.
mod ui;
use crate::{
    appearance, cli,
    commands::{self, Action},
    directory,
    document::DiskFile,
    editor,
    io::{IoWorker, Job, Reply},
    markdown, native_editor,
    navigation::Navigator,
    search, storage,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Preview {
    disk: DiskFile,
    text: String,
    metadata: editor::Metadata,
    generation: u64,
}
struct Picker {
    navigator: Navigator,
    directories: directory::Directories,
    io: IoWorker,
    search: search::Search,
    native: native_editor::Editors,
    markdown: markdown::Markdown,
    settings: storage::Settings,
    bindings: Vec<commands::Binding>,
    preview: Option<Preview>,
    candidate: Option<PathBuf>,
    directory: bool,
    result: Arc<Mutex<Option<PathBuf>>>,
    allow_close: bool,
    choosing: bool,
    completion: crate::picker_state::Completion,
    path: String,
    filter: String,
    status: String,
    token: u64,
    search_id: u64,
    query: String,
    content: bool,
    scope: Option<PathBuf>,
    last_scope: PathBuf,
    hits: Vec<search::Hit>,
    search_state: String,
    search_open: bool,
    read: bool,
    palette: bool,
    palette_query: String,
}
impl Picker {
    fn new(
        cc: &eframe::CreationContext<'_>,
        request: &cli::Request,
        result: Arc<Mutex<Option<PathBuf>>>,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        appearance::install(&ctx, appearance::Density::Comfortable);
        crate::macos::guard_native_quit(&ctx);
        let settings = storage::Settings::load().unwrap_or_default();
        ctx.set_zoom_factor(settings.ui_scale);
        ctx.set_theme(if settings.system_theme {
            egui::ThemePreference::System
        } else if settings.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let initial = request.folder.clone().unwrap_or(home.clone());
        let scope = request.scope.clone().unwrap_or(initial.clone());
        let bindings = commands::bindings(&settings.keymaps)
            .unwrap_or_default()
            .into_iter()
            .map(|mut b| {
                if b.action == Action::Finish {
                    b.action = Action::Choose;
                }
                b
            })
            .filter(|b| allowed(b.action))
            .collect();
        let native = native_editor::Editors::new(ctx.clone(), {
            let window = cc.winit_window().cloned();
            Arc::new(move || {
                if let Some(window) = &window {
                    window.request_redraw();
                }
            })
        });
        native.menu_label(
            Action::Choose,
            if request.directory {
                "Choose Current Folder"
            } else {
                "Choose File"
            },
        );
        native.menu_label(Action::Cancel, "Cancel Picker");
        Self {
            navigator: Navigator::new(home, initial.clone()),
            directories: directory::Directories::new(ctx.clone()),
            io: IoWorker::new(ctx.clone()),
            search: search::Search::new(ctx.clone()),
            native,
            markdown: markdown::Markdown::new(ctx),
            settings,
            bindings,
            preview: None,
            candidate: None,
            directory: request.directory,
            result,
            allow_close: false,
            choosing: false,
            completion: Default::default(),
            path: initial.display().to_string(),
            filter: String::new(),
            status: String::new(),
            token: 0,
            search_id: 0,
            query: String::new(),
            content: false,
            scope: request.scope.clone(),
            last_scope: scope,
            hits: vec![],
            search_state: String::new(),
            search_open: false,
            read: false,
            palette: false,
            palette_query: String::new(),
        }
    }
    fn scope(&self) -> PathBuf {
        self.scope.clone().unwrap_or(self.navigator.current.clone())
    }
    fn preview(&mut self, path: PathBuf) {
        self.token += 1;
        self.candidate = Some(path.clone());
        self.status = "Loading read-only preview…".into();
        self.io.send(Job::Open(path, self.token, true));
    }
    fn choose(&mut self) {
        let path = if self.directory {
            Some(self.navigator.current.clone())
        } else {
            self.candidate
                .clone()
                .or_else(|| self.navigator.selection())
        };
        if let Some(path) = path {
            if !self.completion.begin(path.clone()) {
                return;
            }
            self.choosing = true;
            self.status = "Checking selection…".into();
            self.io.send(Job::Choose(path, self.directory));
        } else {
            self.status = "Select a file, then Choose File.".into();
        }
    }
    fn cancel(&mut self, ctx: &egui::Context) {
        self.completion.cancel();
        *self.result.lock().unwrap() = None;
        self.allow_close = true;
        crate::macos::close_window(ctx);
    }
    fn command(&mut self, action: Action, ctx: &egui::Context) {
        use Action::*;
        if self.choosing && action != Cancel {
            return;
        }
        match action {
            Choose => self.choose(),
            Cancel | Abort | Quit => self.cancel(ctx),
            Home => self.navigator.home(),
            Parent => self.navigator.up(),
            Refresh => self.navigator.refresh(),
            Files | Text => {
                self.content = action == Text;
                self.search_open = true;
            }
            Scope => {
                self.scope = if self.scope.is_some() {
                    None
                } else {
                    Some(self.navigator.current.clone())
                }
            }
            GoScope => {
                let scope = self.scope();
                self.navigator.navigate(scope);
            }
            Read => {
                self.read = !self.read;
                if self.read {
                    self.markdown.refresh();
                }
            }
            Palette => {
                self.palette = true;
                self.palette_query.clear();
            }
            CopyPath => {
                let path = self.candidate.as_ref().unwrap_or(&self.navigator.current);
                self.status = crate::macos::copy_path(path)
                    .err()
                    .unwrap_or_else(|| "Path copied".into());
            }
            _ => {}
        }
    }
    fn flush(&mut self) {
        self.navigator.cancel_inactive();
        self.directories.retain(&self.navigator.requested_folders());
        for path in self.navigator.take_requests() {
            match self.directories.request(path.clone()) {
                Ok(id) => self.navigator.begin(path, id),
                Err(e) => self.navigator.receive(path, Err(e)),
            }
        }
    }
}
fn allowed(action: Action) -> bool {
    matches!(
        action,
        Action::Choose
            | Action::Cancel
            | Action::Quit
            | Action::Home
            | Action::Parent
            | Action::Refresh
            | Action::Files
            | Action::Text
            | Action::Scope
            | Action::GoScope
            | Action::Read
            | Action::Palette
            | Action::CopyPath
    )
}
impl eframe::App for Picker {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            self.cancel(ctx);
        }
        for _ in 0..2 {
            let Ok(batch) = self.directories.replies.try_recv() else {
                break;
            };
            self.navigator.batch(batch);
            ctx.request_repaint();
        }
        for _ in 0..8 {
            let Ok(reply) = self.io.replies.try_recv() else {
                break;
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint();
            match reply {
                Reply::Open(path, token, result) if token == self.token => match result {
                    Ok((disk, text)) => {
                        self.candidate = Some(path);
                        let mut metadata = editor::Metadata::default();
                        metadata.update(&text, token);
                        self.preview = Some(Preview {
                            disk,
                            text,
                            metadata,
                            generation: token,
                        });
                        self.status =
                            "Read-only preview. Choose explicitly returns the selection.".into();
                    }
                    Err(e) => {
                        self.preview = None;
                        self.status = format!(
                            "Preview unavailable: {e}. An existing file can still be chosen."
                        );
                    }
                },
                Reply::OpenDirectory(path, token) if token == self.token => {
                    self.navigator.navigate(path);
                    self.preview = None;
                    self.candidate = None;
                }
                Reply::Visit(path, true) => {
                    self.navigator.navigate(path);
                    self.candidate = None;
                }
                Reply::Visit(path, false) => self.preview(path),
                Reply::Chosen(path, Ok(())) => {
                    if !self.completion.complete(&path, true) {
                        continue;
                    }
                    *self.result.lock().unwrap() = Some(path);
                    self.allow_close = true;
                    crate::macos::close_window(ctx);
                }
                Reply::Chosen(path, Err(e)) => {
                    self.completion.complete(&path, false);
                    self.choosing = false;
                    self.status = format!("Selection unavailable: {e}");
                }
                _ => {}
            }
        }
        while let Ok(result) = self.search.replies.try_recv() {
            if result.generation == self.search_id {
                self.hits = result.hits;
                self.search_state = result.state;
            }
        }
        let scope = self.scope();
        if scope != self.last_scope {
            self.last_scope = scope;
            self.search_id = self.search.cancel();
            self.hits.clear();
            self.search_state = "Cancelled — scope changed".into();
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.render(ui, frame);
    }
}
pub fn run(request: cli::Request) -> eframe::Result {
    let result = Arc::new(Mutex::new(None));
    let output = result.clone();
    let directory = request.directory;
    let nul = request.nul;
    let options = crate::window::options();
    let outcome = eframe::run_native(
        "Glimpse Picker",
        options,
        Box::new(move |cc| Ok(Box::new(Picker::new(cc, &request, result)))),
    );
    if let Err(e) = outcome {
        eprintln!("Picker failed: {e}");
        std::process::exit(2);
    }
    if let Some(path) = output.lock().unwrap().take() {
        use std::{io::Write, os::unix::ffi::OsStrExt};
        let mut stdout = std::io::stdout().lock();
        let result = if nul {
            stdout
                .write_all(path.as_os_str().as_bytes())
                .and_then(|_| stdout.write_all(&[0]))
        } else {
            writeln!(stdout, "{}", cli::selection_json(&path, directory))
        }
        .and_then(|_| stdout.flush());
        if let Err(e) = result {
            eprintln!("Cannot write picker result: {e}");
            std::process::exit(2);
        }
        std::process::exit(0);
    }
    std::process::exit(1);
}
