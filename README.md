# Glimpse

Glimpse is a native Rust file navigator and UTF-8 editor using GPUI. The Linux port is under development. macOS remains a target but this revision has not been compiled or exercised there. No WebView is used.

The [product contract](glimpse.md) defines the intended behavior. The [core design](docs/core-design.md) describes the port and its remaining gaps. Earlier [qualification results](docs/qualification.md) describe the retired macOS prototype, not this GPUI implementation.

## Build

Rust 1.95 is pinned. On Fedora, install the native development dependencies:

```sh
sudo dnf install fontconfig-devel freetype-devel libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel wayland-devel vulkan-loader-devel
CARGO_BUILD_JOBS=4 cargo run --release --locked -- /path/to/folder
```

A Wayland or X11 desktop and a Vulkan driver are required on Linux. GPUI file dialogs use the desktop portal; install a portal backend supporting file selection. macOS builds require Apple's Command Line Tools.

```sh
cargo fmt --check
CARGO_BUILD_JOBS=4 cargo clippy --locked --all-targets -- -D warnings
CARGO_BUILD_JOBS=4 cargo test --locked --workspace
```

Set `GLIMPSE_TRACE=1` to emit monotonic edit/save event timings to stderr. Events identify documents and generations without logging their contents. These timestamps describe application and worker handling, not physical input-to-present latency. `GPUI_WORK_TRACE=1` additionally records monotonic start and duration in nanoseconds for UI updates, input dispatch, effects, frame construction and renderer submission. Updates include nested work: merge overlapping intervals rather than adding their durations. Renderer submission is not physical presentation; disable tracing for idle CPU measurements.

Linux tests require util-linux `unshare` and `mount`, with user and mount namespaces available. Bind-mount fixtures run in a private namespace.

## Workflow

The left pane lists one directory. Click folders to enter them and files to preview them. Double-click keeps a tab; editing pins a preview. Back, Forward and Parent change location.

The path bar’s ancestor buttons navigate directly. Ctrl-L enters a path, Tab completes it, and Escape cancels. Dropping a file opens it; dropping a folder navigates there. Horizontal navigator gestures and Back/Forward mouse buttons traverse history. Ctrl-Shift-E focuses the navigator, and arrow keys and Enter navigate its rows. Ctrl-O opens a native dialog. Unusual names copy in a lossless escaped form that Location can reopen.

Files and Text search recursively under the displayed folder; scope can be pinned independently of location. Search This Folder provides a temporary override. Index This Folder builds a session filename snapshot with visible stale, rebuild and removal controls; text search reads saved files. Results support arrow keys and Enter, and retain coverage and exclusions beside the query.

Filter affects only the current directory. The navigator reports slow or failed reads; Stop Reading keeps existing rows available, and Refresh resumes listing.

Ctrl-N creates a draft. Ctrl-S saves; Ctrl-Shift-S opens Save As; Ctrl-Alt-S opens Save Copy. Ctrl-F searches the current buffer with case and whole-word options. Replacement is disclosed on request; Replace and Replace All have separate Undo steps. Escape returns to the selected match. Ctrl-G goes to a line. Ctrl-Shift-P opens commands, Ctrl-comma opens preferences, Ctrl-W closes a tab and Ctrl-Q quits. The corresponding Command modifier applies on macOS. Read switches the current buffer to a read-only Markdown presentation; Edit returns to the source position. Local raster images are bounded, remote images are omitted, raw HTML is visible source, and relative file links open from the document’s folder.

Save validation, no-clobber creation/rename and recovery use the existing domain workers. Ordinary saves refuse detected external changes. Clean external changes reload with an undo step; dirty buffers retain local text and offer Reload or Save Copy. Dirty close exposes Save, Discard and Cancel. Linux removal uses freedesktop Trash, never permanent deletion. Close an open file before rename or Trash.

Settings and recovery use `$XDG_DATA_HOME/glimpse` or `~/.local/share/glimpse` on Linux and `~/Library/Application Support/Glimpse` on macOS. `GLIMPSE_DATA_DIR` isolates test sessions.

## CLI

```sh
glimpse /existing/folder /existing/file.txt
glimpse open --scope /folder --line 12 --column 3 --wait -- /file.txt
glimpse pick --directory --format json /starting/folder
```

The CLI parser, private lifecycle socket and lossless picker output are retained. Save does not finish a waiting request: use Finish Editing Requested Files. Save As for a waiting requested file asks for an explicit abandonment decision; accepting it returns failure to that caller. See the core design for differences from the previous prototype and unqualified behavior.
