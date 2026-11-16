# Core implementation design

[Glimpse's product contract](../glimpse.md) governs scope and interaction. This document describes the macOS core and the boundaries its native interface must preserve. Windows and Linux are not qualified.

## Execution and ownership

The UI owns document IDs, focus, selection, navigation, request generations and decisions. Storage, enumeration, search and parsing run on bounded workers. Platform text editing stays on AppKit's main thread; Glimpse's shell and Markdown renderer use Rust/egui without a WebView.

| Work | Resources per session | Policy |
| --- | --- | --- |
| Navigation listings | Two workers; 64 queued requests; 16 reply batches | Stream entries, reject stale path generations, sort and merge batches, virtualize visible rows. Cache at most 64 listings / 32 MiB; each listing stops at 100,000 entries / 16 MiB and reports partial coverage. |
| Path completion | Separate two-worker directory service | Enumerate only on completion request; show at most 32 matches and cancel obsolete work. |
| Document reads | Two workers; 32 queued requests | Bound files to 10 MiB; check cancellation between 64 KiB reads. An obsolete preview cannot replace a newer selection. Explicit opens can finish in background without stealing focus. |
| File mutations | One worker; 16 queued requests | Serialize no-clobber creation, rename, OS Trash and validated saves. |
| Recovery and preferences | One maintenance worker; coalesced draft updates | Persist exact generations away from the frame path. Active recovery sessions retain exclusive ownership. |
| Recursive search | One worker; latest-request mailbox; eight reply snapshots | Stream ranked results, cancel replaced queries, stop at explicit coverage limits. |
| Syntax | One worker; latest-request mailbox | Use syntect with packaged two-face grammars. Check a 200 ms budget between lines, cap at 30,000 spans or a 32 KiB line, and label partial highlighting. A grammar call already in progress cannot be interrupted. Apply native color spans in bounded slices. |
| Markdown | One preparer and two raster workers | Reading text does not wait for image I/O. Bound decoding and retain only the current reading presentation. |
| External observation | Dedicated watcher control thread | Notifications request complete revalidation; they are not authoritative file versions. |

OS and toolkit threads are additional to these counts. Cancellation abandons obsolete results; it cannot interrupt every filesystem call. A stalled volume consumes a bounded worker rather than creating replacement threads indefinitely. No periodic indexer or idle repaint loop is required.

Directory paths, selected rows and document bindings use native OS path values. A displayed label is never a filesystem identity. Recursive search respects ignore files by default, includes dotfiles, excludes version-control metadata and does not follow directory symlinks or nested mounts. Settings can deliberately include ignored/metadata paths and add ignore-style exclusions. Content limits and incomplete coverage remain visible.

## Documents and saves

Each document has an ID, original pathname, resolved target, observed identity and bytes, current text, last saved text, and a monotonically increasing generation. UTF-8 BOM, newline sequences and final-newline state remain unchanged unless the user edits them. Unsupported data is rejected rather than decoded lossily into an editable partial document.

Save captures a specific candidate. Before replacement it persists that candidate and the observed disk base, stages beside the target, preserves supported metadata, and revalidates the full destination bytes, identity and symlink binding. It then uses atomic replacement. Hard-linked files and unsupported metadata cases fail with alternatives. A detected change never becomes an ordinary overwrite.

An overwrite decision authorizes the displayed disk version only. Another change invalidates it. Save As changes the binding only after success; Save Copy keeps the binding and requires a separate destination. First creation and rename use no-clobber operations, including when another writer creates the destination after the dialog. Removal uses OS Trash; permanent deletion is outside the product.

A save acknowledgement marks only its captured text as saved. Edits made while it ran remain dirty. External reads carry document generations so an older result cannot refresh a newer edit or binding. Identical disk contents update identity without replacing text. A clean changed buffer refreshes after composition/selection activity, retaining the old text as an undo step. Dirty changes remain explicit conflicts with compare, reload, copy and version-specific overwrite actions.

Atomic replacement is not compare-and-swap against unrelated writers. Another application can write after the final validation; Glimpse does not promise recovery of every unseen concurrent version.

## Recovery and preferences

Recovery uses owner-private, transactional records and a locked session directory. Records include exact local text, observed bytes, binding and generation. An abandoned record transfers into the importing session without leaving a second automatic-restoration copy. Live sessions cannot be imported. Restored records remain dirty until resolved. Older background writes cannot replace a newer record or resurrect an acknowledged saved/discarded generation.

Successful save snapshots move into resolved history. Only resolved history expires: seven days or a 128 MiB budget. Unresolved drafts do not expire. Recovery management exposes records, sizes, restoration and OS Trash controls. Recovery failures block normal replacement; Retry, a copy elsewhere, and an explicit one-attempt Save Without Recovery retain destination validation. An unresolved content conflict cannot use the ordinary recovery bypass.

Preferences are declarative and versioned. Writes validate ranges, shortcut conflicts and exclusion syntax, compare the last observed settings bytes, keep a previous valid copy and replace atomically. An external settings edit requires Reload. An invalid file retains the previous valid settings where available and produces a useful error. Repository-local configuration is not executed.

The command registry supplies native menus, shortcut routing and the command palette. Clipboard and Undo/Redo shortcuts remain reserved for text editing. Source typography, line numbers, syntax, wrapping, indentation, visibility, search filters and limits, shortcuts, and recovery controls have functional settings.

## Editor and reading presentation

The current macOS adapter uses NSTextView for native editing, clipboard, undo, text input and accessibility inside the Rust-rendered document pane. It retains a view per open document to preserve caret, scroll and undo state. The shell is not being replaced with a new framework. Comparative evidence and the large-file qualification gate remain necessary: component availability does not establish correctness or performance.

Line numbers use cached line starts and visible native layout. Syntax failure leaves editable plain text. Go to Line uses one-based Unicode scalar columns and clamps to grapheme boundaries. Find operates on the current buffer; recursive content search operates on saved bytes and revalidates hits before line activation.

Markdown source remains authoritative. Reading uses egui_commonmark/pulldown-cmark, with CommonMark, tables, task lists, strikethrough and angle-bracket autolinks; the component also recognizes footnotes and definition lists. Raw HTML remains source text. Reading never edits task markers or executes document content. Links require activation and resolve relative to the document. Remote images are not fetched.

Local raster images are limited to 8 MiB compressed, eight megapixels, 8192 pixels per side, 32 images and a 32 MiB decoded cache. Animation shows a static decoded image. Large reading documents use pages at block boundaries. A block exceeding 64 KiB receives an explicit bounded source preview and an Edit Source alternative; it is never substituted for, or saved over, the complete source.

## CLI sessions

Each request owns its GUI lifecycle. A non-waiting editor invocation returns only after the requested target has reached a usable rendered frame. A private socket carries readiness and completion status separately from document data. A waiting caller can return when its original destinations finish while unrelated GUI work remains open. Startup has a 30-second failure timeout; that timeout does not relax the 250 ms performance target.

Requested destinations are tracked separately from document bindings, including aliases sharing one buffer. Finish revalidates original path bindings and bytes. Save alone does not finish a request. Save Copy retains it. Save As to another destination requires an explicit abandonment decision and cannot return request success. Discard, abandonment and failure have distinct exit statuses.

A picker is a separate read-only Glimpse session with navigation, preview and scoped search. Previewing does not return a result. Choose validates one existing file or folder and outputs lossless native path data as versioned JSON or Unix bytes followed by NUL. Cancel writes no result. A picker does not restore drafts or modify the normal editing session.

## Acceptance budgets

The targets remain: 120 Hz file-view interaction (8.33 ms per frame), under 250 ms to a usable app frame, aspirational 50 MB idle RAM excluding text, and 0.1% idle CPU. Measure total resident memory as well as clearly identified content costs; do not subtract guessed text or allocator costs. Report meaningful idle intervals and representative large directories.

A prepared UI pass or synthetic CPU benchmark does not establish physical input-to-present latency. Startup instrumentation waits for a completed prior GUI frame and the initial native text draw when a document is present. Compositor/display delivery remains a separate measurement. Report measured misses and unverified platform behavior explicitly; do not turn these targets into claims of qualification.

## Filename snapshots and visible folders

The search worker can build one filename snapshot after explicit Index This Folder activation. It retains native paths for one scope/filter set, bounded by 100,000 visited entries and 16 MiB of path bytes. Snapshots are process-local and replaced on rebuild or released on removal. Query changes cancel obsolete work. A watcher installed for the explicitly selected scope marks its snapshot stale on relevant filesystem changes; it never triggers another crawl. Rebuild remains explicit. Queries clone the shared path-array handle before ranking, keeping index status/removal responsive. Saved-content search bypasses the snapshot. Visible-folder watchers refresh directory listings independently of document observation and filename snapshots.

Current acceptance uses the newer sub-100 ms opening, below-100 MB total RAM and 0.1% idle CPU aspirations documented in the product contract, with active CPU proportional to the interaction. Historical idle targets are not substitutes for active measurements.

Live listing refresh keeps the previous rows until the first replacement batch, retaining path-based selection and scroll offsets. Navigation and nonempty live batches start a 140 ms settling wash; input is never delayed and rows do not translate. Repaints stop when that interval ends, and Reduce motion skips it. Notifications ignore access-only/other events and drain queued changes into one refresh request.

For source lines above 16 KiB, the macOS adapter configures a horizontally scrolling text container before inserting the string. Wrapping, ruler glyph queries and current-line glyph queries pause for that document; full text and validated saving remain available. This reduces measured layout work but does not qualify the 10 MiB long-line responsiveness or memory envelope.
