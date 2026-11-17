mod gui;
use glimpse::{cli, lifecycle};

static EXIT_STATUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match cli::informational(&args) {
        Some("help") => {
            println!(
                "Glimpse\nUsage: glimpse [PATH ...]\n       glimpse open [--scope DIR] [--line N] [--column N] [--wait] -- FILE ...\n       glimpse pick [--directory] [--format json|nul] [START_DIR]"
            );
            return;
        }
        Some("version") => {
            println!("glimpse {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }
    let request = cli::parse(args.clone(), std::env::current_dir().unwrap()).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    if std::env::var_os("GLIMPSE_GUI").is_none() {
        lifecycle::launch(&args, request.wait, request.pick);
    }
    if let Err(e) = lifecycle::connect() {
        eprintln!("{e}");
        std::process::exit(2);
    }
    gui::run(request);
    std::process::exit(EXIT_STATUS.load(std::sync::atomic::Ordering::Relaxed));
}
