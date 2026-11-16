#![cfg_attr(not(target_os = "macos"), allow(unused))]
#[cfg(not(target_os = "macos"))]
compile_error!("This first Glimpse prototype supports macOS only.");

mod app;
mod appearance;
mod cli;
mod closing;
mod commands;
mod comparison;
mod directory;
mod document;
mod editor;
mod fileops;
mod io;
mod lifecycle;
mod macos;
mod markdown;
mod native_accessibility;
mod native_editor;
mod navigation;
mod observe;
mod paths;
mod perf;
mod picker;
mod picker_state;
mod reservations;
mod search;
mod storage;
mod syntax;
mod window;

pub(crate) use paths::name;
use std::path::Path;

static EXIT_STATUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
fn main() -> eframe::Result {
    let started = std::time::Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if cli::informational(&args) == Some("help") {
        println!(
            "Glimpse {}\nUsage: glimpse [PATH ...]\n       glimpse open [--scope DIR] [--line N] [--column N] [--wait] -- FILE ...\n       glimpse pick [--directory] [--format json|nul] [--scope DIR] [START_DIR]\nUTF-8 editing up to 10 MiB. Use -- before paths named like options or commands.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if cli::informational(&args) == Some("version") {
        println!("glimpse {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let cwd = std::env::current_dir().unwrap();
    let desktop = args.is_empty() && cwd == Path::new("/");
    let cwd = if desktop {
        std::env::home_dir().unwrap_or(cwd)
    } else {
        cwd
    };
    let mut request = cli::parse(args.clone(), cwd).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    request.desktop = desktop;
    if std::env::var_os("GLIMPSE_GUI").is_none() {
        lifecycle::launch(&args, request.wait, request.pick);
    }
    if let Err(error) = lifecycle::connect() {
        eprintln!("Cannot connect lifecycle channel: {error}");
        std::process::exit(2);
    }
    if request.pick {
        return picker::run(request);
    }
    let path = request
        .folder
        .clone()
        .or_else(|| {
            request
                .files
                .first()
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
        .unwrap_or_else(|| std::env::home_dir().unwrap());
    let options = window::options();
    eframe::run_native(
        "Glimpse",
        options,
        Box::new(move |cc| {
            let mut app = app::Glimpse::new(cc, path);
            app.configure_request(request, started);
            Ok(Box::new(app))
        }),
    )
    .unwrap_or_else(|error| {
        eprintln!("GUI failed: {error}");
        std::process::exit(2)
    });
    std::process::exit(EXIT_STATUS.load(std::sync::atomic::Ordering::Relaxed));
}
