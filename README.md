# Glimpse

A macOS file navigator and UTF-8 editor built with Rust, egui/eframe and an embedded AppKit source editor. Markdown uses a native Rust renderer, without a WebView. This development build implements the core workflow; it is **not performance-qualified for release**. The [product contract](glimpse.md), [core design](docs/core-design.md), [UI design](docs/ui-design.md) and [qualification results](docs/qualification.md) describe its scope and limits.

## Build and run

Install Rust through [rustup](https://rustup.rs/) and Apple's Command Line Tools (`xcode-select --install`). The repository pins Rust 1.95.

```sh
cargo run --release --locked -- /path/to/folder
./scripts/package-macos.sh
open build/Glimpse.app
```

The ad hoc signed bundle is for local use, without notarization. Only macOS on Apple Silicon has been exercised; Windows and Linux are unsupported by this build.

## Navigation and editing

Browse one directory list with a retained selection and vertical scroll position per folder. Opening a file keeps that list beside the document. Back/Forward and horizontal gestures navigate directory history; one gesture advances at most one location, including its momentum. Parent reaches `/`; Home and Volumes remain in the folder menu. Clicking a breadcrumb navigates there directly. Click the path bar's blank space or press Command-L to enter a path; Tab offers completions. Local filtering, native dialogs and file drops provide additional access. Search scope follows Location unless explicitly pinned; opening a search result does not move Location.

A file click opens a replaceable clean preview tab. Editing pins it; explicit opening keeps it. Native source views retain caret, scrolling and undo across tab switches. Standard selection, clipboard, Undo/Redo, indentation, wrapping, line numbers and Go to Line are available. Command-F opens a compact floating Find panel with live match counts; its disclosure reveals Replace and Replace All. Case sensitivity, whole words and a labelled Go to Line field remain available. Escape or Close dismisses Find and restores editor focus. Status shows line/column, encoding, newline style and highlighting state. Syntax covers Markdown, Rust, Python, JavaScript, TypeScript, Go, C/C++, shell, JSON, YAML, TOML, HTML and CSS. Unknown formats remain plain text; highlighting work limits are visible.

Command-N creates a draft, Command-O opens, Command-S saves, Command-F finds, Command-W closes and Command-Q quits. Dirty close/quit offers Save, Discard and Cancel. The command palette (Command-Shift-P), menus and Settings expose commands and configurable shortcuts. Standard clipboard and Undo/Redo shortcuts are reserved for editing.

## Markdown

Read / Edit switches between authoritative source and a read-only presentation of the current buffer, including unsaved text. The dialect includes CommonMark, tables, task lists, strikethrough, angle-bracket autolinks, footnotes and definition lists. Raw HTML is displayed as source; document content never executes. Task markers are read-only. Links require activation; relative links resolve beside the document. Remote images are not fetched.

Local raster images have compressed-size, dimension, count and decoded-cache limits. Large documents use pages at block boundaries. A block over 64 KiB shows an explicitly bounded source preview and an Edit Source action; it never replaces the complete editable document. See [the reading contract](docs/core-design.md#editor-and-reading-presentation) for limits.

## File safety and recovery

Editable input is regular UTF-8 text, with or without BOM, up to 10 MiB. Binary/control data, unsupported encodings, special files and larger files receive an explanation and an external-open alternative. The size limit is an acceptance bound, **not a large-file responsiveness guarantee**. Untouched saves preserve exact bytes. Edited buffers retain existing newline sequences and BOM; newly typed breaks use LF.

Save stages beside the target, preserves supported macOS metadata, syncs and atomically replaces after checking target identity, symlink binding and full contents. Hard links and unsupported ownership/metadata stop ordinary saving. An unrelated writer can still act after the final check: this is not compare-and-swap or a universal power-loss guarantee.

Save As rebinds only after success; Save Copy retains the original binding. Existing destinations require an explicit decision about the observed version and are validated again before replacement. Clean open aliases are reconciled; dirty aliases block replacement. Named file/folder creation and single-item rename never clobber an existing entry. Close an open file before renaming or moving it to macOS Trash. No permanent-delete command exists. Copy Path, Reveal in Finder and Open Externally are available.

Filesystem notifications and focus return trigger revalidation. Clean changed text refreshes with an undo step. Dirty text remains intact and offers Compare, explicit Reload, Save Copy or replacement of the specifically observed disk version. A later change invalidates that overwrite decision.

Private recovery records contain exact local candidates, observed disk bytes, bindings and generations. Live sessions are locked against import. Restoring an abandoned record transfers ownership once and opens a dirty draft without automatically writing its old destination. Save/discard clears only acknowledged generations. Resolved history expires after seven days or above 128 MiB; unresolved drafts do not expire. Recovery management shows usage, partial inventory, history/candidate records, restoration, observed-disk snapshots and OS Trash actions. Recovery failures offer Retry, Cancel, a copy elsewhere or an explicit one-attempt save bypass that retains destination validation. Forced termination can lose edits that have not reached recovery storage.

Settings and recovery normally live under `~/Library/Application Support/Glimpse`; `GLIMPSE_DATA_DIR` isolates disposable test state. Settings validate font names, ranges, shortcuts and search exclusions, detect concurrent changes and keep a previous valid copy. Categories cover appearance/density/scale, source typography, wrap/indentation/syntax/line numbers/current-line shading, Markdown typography, navigation, search limits/filters, keymaps and recovery management.

## Bounded work and search

Directory enumeration, document reads, mutations and maintenance have separate bounded workers. Listings and search results stream; obsolete generations are cancelled or ignored. Rows are virtualized, and directory caches have count/byte budgets. Blocking filesystem calls cannot always be interrupted, but do not create unbounded replacement workers. There is no periodic indexer or idle repaint loop.

Filename search ranks exact/prefix/substring and subsequence matches within the displayed recursive scope. Content search reads saved disk text; document Find uses unsaved buffer text. Search stops at 100,000 entries, 1,000 hits and a configurable 1–10 MiB per content file. Limited/unreadable coverage is Partial. Ignore files apply by default; version-control metadata, directory symlinks and nested mounts are excluded unless the relevant setting permits metadata. Results revalidate saved bytes and dirty-buffer state before activating a line.

## CLI

```sh
glimpse /existing/folder /existing/file.txt
glimpse open --scope /folder --line 12 --column 3 --wait -- /file.txt
glimpse pick --directory --format json /starting/folder
glimpse -- ./open
```

Relative paths resolve from the caller's working directory; missing paths create nothing and exit 2. Shorthand accepts at most one folder; `open` accepts files. Non-waiting opens launch a separate GUI and return after its usable-frame acknowledgement, with empty stdout. This acknowledgement includes initial native source draw readiness, not physical display presentation. Line/column positions are one-based Unicode scalar positions, clamped to grapheme boundaries with a notice.

A picker is a separate read-only Glimpse window with navigation, preview and search. Choose validates one existing file or folder, then returns versioned JSON containing native Unix path bytes in base64, or exact bytes followed by NUL with `--format nul`. Cancel writes no result and exits 1; failure exits 2.

`--wait` owns the original requested destinations, including aliases sharing one buffer. Save alone does not finish. Finish Editing revalidates the requested files; unrelated tabs can remain open after the caller returns. Success exits 0; Discard, Abort or Save As abandonment exits 1; failure or premature GUI termination exits 2. Save Copy retains the request. Save As to another destination requires explicit abandonment.

## Verification

See the [feature matrix](docs/feature-matrix.md) for implemented workflows and remaining qualification.

Use the [native evaluation guide](docs/native-evaluation.md) for computer-use walkthroughs, Mac reference comparisons and iterative UI/UX review.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --release --locked benchmark_ten_thousand_rows -- --ignored --nocapture
./scripts/package-macos.sh
```

Tests cover no-clobber operations, metadata and byte preservation, save conflicts/aliases, recovery generations/live sessions/failure paths, search cancellation/scope/limits, settings persistence, CLI parsing and lifecycle completion. Actual native checks additionally exercise file operations, Save/As/Copy races, external refresh/undo, crash recovery, picker JSON/NUL/cancel, wait/Finish ownership, search activation and Unicode/grapheme editing.

Startup remains above the current 100 ms aspiration, total RAM exceeds 100 MB, and active CPU exceeds 0.1%. Long-line editing still fails the intended responsiveness envelope; lines above 16 KiB temporarily disable wrapping, line-number painting and current-line shading, with an explicit status message. The complete source remains editable. Physical 120 Hz input-to-present, full VoiceOver operation and real IME preedit/composition-and-commit remain unverified; Unicode paste and dead-key input do not qualify IME. Startup and idle measurements, editor comparisons and specific limitations are recorded in [qualification results](docs/qualification.md). Targets remain unchanged.

### Navigation and filename snapshots

Favorites live in the directory header's compact star menu. Pin the current folder or active file there; remove pins in Settings. Frequent & recent remains in the folder menu and ranks document-opening visits with a seven-day half-life; repeated opens within 30 minutes count as one visit. Folder traversal alone does not add entries. The visible directory and open document parents are observed for filesystem changes.

The toolbar search field accepts the query directly. Results appear in a panel anchored below it, preserving the directory list and document. Typing or Return searches the displayed scope. Files/Text chooses the operation, and Options holds scope and filename-index actions. Escape dismisses search.

In Files search, open **Options → Index This Folder** to cache filenames for that scope. This is opt-in, session-only, limited to 100,000 visited entries and 16 MiB of native path bytes, with at most 1,000 displayed hits. Queries over the snapshot update as typed. A watcher marks the selected scope's snapshot changed on disk without rebuilding it. Use **Rebuild Filename Index** for current filenames or **Remove Index** to release both snapshot and watcher. The snapshot respects the chosen filters and does not follow directory symlinks or cross nested mounts. Text search still reads saved files. No automatic whole-disk index is built.

Set `GLIMPSE_BACKGROUND=1` for a nonactivating native launch during measurement. This does not qualify visible input-to-present performance.

### Frame presentation

Glimpse uses Metal with low-latency presentation on macOS. Vertical synchronization
is disabled to reduce the age of displayed input; this can permit tearing. Launch
with `GLIMPSE_VSYNC=1` to restore synchronized presentation.
