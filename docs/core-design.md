# Core implementation design

[Glimpse's product contract](../glimpse.md) governs the product. This revision replaces the egui/eframe and embedded AppKit application with GPUI 0.2.2. Linux is the current verification target; macOS compilation and interaction are unverified.

## Boundaries

The GPUI shell owns document IDs, tabs, focus, navigation history, search scope and user decisions. Disk files, save validation, recovery, search, directory enumeration, CLI parsing and lifecycle remain ordinary Rust modules. Worker completion uses a small renderer-independent callback, connected to a bounded GPUI wake channel. The domain modules are exposed through the library target and can be tested without opening a window. The runtime does not use egui. Retained egui presentation regression tests are test-only; they do not qualify the new interface.

Document reads use two bounded workers, mutations one, and maintenance one. Directory listings use two workers and stream bounded batches. Recursive search and syntax each use a latest-request mailbox. Notifications trigger full document validation; they are not authoritative versions. No periodic scan or repaint loop is intended.

## Editor choice

The editor uses gpui-component 0.5.1 InputState in code-editor mode, retaining one entity per document. Its rope buffer, visible-line rendering, selection, clipboard, undo and GPUI input-method handling provide a smaller implementation surface than writing those primitives again. This is a provisional choice: a rope does not establish large-file performance, long-line responsiveness, IME correctness or accessibility.

The component is vendored under its Apache-2.0 license for externally supplied highlight spans, a steady caret, live indentation preferences, bounded visible-row shaping, and keyboard-operable menus. Line metadata retains lengths instead of individual rope copies. Wrapping is prepared before first layout; off-screen wrapped rows retain byte ranges and geometry without shaped glyphs. These changes reduce layout allocations but do not eliminate full-line wrapping or whole-buffer extraction. The existing bounded syntect/two-face worker remains responsible for syntax. Generation checks reject obsolete results; UTF-16 grammar offsets become UTF-8 byte ranges at the presentation boundary. The other vendored patch corrects xattr 0.2.3's Linux errno name for aarch64 in GPUI's transitive archive dependency. Neither patch changes file-save validation.

The component's default text operations preserve the authoritative buffer. Performance qualification must cover 10 MiB documents, long lines, graphemes, real IME composition and clipboard. None of those requirements is relaxed by choosing this component.

## File safety and services

DiskFile retains observed identity, symlink binding and bytes. Save stages beside the target, preserves supported metadata, syncs, validates and atomically replaces. Hard links, unsupported ownership/metadata and changed destinations stop saving. Atomic replacement is not compare-and-swap against unrelated writers. External observation retains the existing binding when a symlink target changes, even when both targets have identical bytes; ordinary Save stays blocked until an explicit resolution.

Save As rebinds after success; Save Copy retains the original binding. Existing destinations require confirmation of an observed version. Destinations already open in another tab must be closed first. Recovery remains mandatory for normal saves. Failure retains local text and reports the error; the earlier one-attempt recovery bypass is not exposed.

Linux rename uses `renameat2(RENAME_NOREPLACE)` and Trash uses the trash crate's freedesktop implementation. Linux saving copies permissions and extended attributes; failures stop replacement. macOS retains `renamex_np`, NSFileManager Trash and `fcopyfile` metadata preservation behind platform configuration. Linux Reveal opens the containing directory with xdg-open. GPUI supplies native portal file dialogs.

Recovery keeps private, locked session records with exact local text and observed disk bytes. Recovery lists storage usage, abandoned drafts, failed-save candidates and resolved history, with keyboard-accessible restoration and Trash controls. Claiming an abandoned draft retains its binding; history and observed snapshots open separate copies. Restoring an already-open original also creates a separate copy. New save candidates identify their owning document, so resolving one document archives only its candidates through the acknowledged generation. Legacy candidates without ownership remain available for explicit management. Closing waits for acknowledgment that the exact recovery generation was resolved. Cleanup failure keeps the document open with Retry and Keep Open actions; Keep Open creates a newer recovery generation. Close decisions retain the named document IDs across tab switches, and Save and Close saves those documents before cleanup. Edits during cleanup cancel that document’s close.

Conflict decisions capture the document, local generation and specific observed disk version. Compare exposes selectable, read-only local and disk text. Reload revalidates the observed version and acknowledges recovery cleanup before replacing the buffer; the old local text remains in Undo and history. Overwrite uses the normal validated-save path against the explicitly observed version. Both operations retain local text on failure.

Requested CLI files complete only after destination validation and recovery cleanup. Clean close can complete a request; closing one of several requested files leaves the others waiting. Dirty Finish offers Save, Discard and Cancel. Save alone keeps waiting; Discard or Reload that abandons requested local edits returns failure. Generation checks, exact disk bases and authoritative buffer comparisons reject obsolete completion and cleanup replies.

## Preferences and layout

Preferences edits a draft until Apply validates and persists it. The worker acknowledges settings writes before the shell adopts the saved values. Cancel retains the running configuration; Reload reads the observed file again. External settings changes block replacement, and each write validates the candidate’s observed bytes rather than a mutable worker-wide baseline. The writer enforces the same 64 KiB limit as the reader. Theme, typography, wrapping, indentation, line numbers, current-line highlighting, syntax, density, hidden files, search filters and shortcut overrides share the stored configuration.

Locations contains explicit pins, frequent/recent folders and mounted folders. Successful document openings update visit ranking; traversal alone does not. The shell allows one settings write at a time and stages visits while Preferences is open. Pin changes become live only after acknowledgment. Linux mount discovery reads the [kernel mount table](https://www.kernel.org/doc/Documentation/filesystems/proc.txt), preserves native path bytes and reports bounded-list truncation. macOS retains a separate `/Volumes` implementation, which is unqualified.

Shell buttons and document tabs use focusable GPUI controls. Preferences contains Tab navigation, scrolls focused fields into view and restores focus on dismissal. The navigator divider accepts pointer dragging and keyboard resizing. Narrow layouts show either navigator or document, use fewer visible toolbar actions and retain the command palette. The width threshold accounts for interface scale. Menus support keyboard opening, focus return and scrolling; labels use bounded ellipses and wrapped tooltips. Duplicate document names use the shortest distinguishing parent suffix, with full paths available on hover. Document subscriptions end when their tabs close. App shortcut matching precedes editor bindings while preserving reserved editing keys. These changes do not establish screen-reader support.

## Presentation gaps and qualification

The product contract and native Linux evaluation determine qualification. The following remain incomplete:

- Breadcrumb/path completion, drops, horizontal navigation gestures and complete contextual command availability. macOS system-menu integration remains unverified.
- Recovery failure alternatives, including the explicit one-attempt Save Without Recovery.
- Filename-index controls. Content-result activation rejects changed disk fingerprints and dirty open buffers.
- The earlier bounded Markdown paging, local raster images, relative-link handling and complete dialect parity. The current read view escapes HTML and omits images, including remote images.
- Large-file/long-line performance, real IME, accessibility, the complete size/theme/settings matrix and macOS behavior. Modal behavior across asynchronous completions still requires qualification.

The new view virtualizes directory rows and stores per-folder selection and scroll state. Syntax work remains bounded, but extracting full buffer text on edits can be costly. Linux native checks qualify only their executed scenarios. Whole-app performance, safety and accessibility require current-build evidence for the complete acceptance matrix.

Save As for a waiting requested file still requires finishing or aborting the request first; its explicit abandonment decision remains incomplete.
