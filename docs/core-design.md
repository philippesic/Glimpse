# Core implementation design

[Glimpse's product contract](../glimpse.md) governs the product. This revision replaces the egui/eframe and embedded AppKit application with GPUI 0.2.2. Linux is the current verification target; macOS compilation and interaction are unverified.

## Boundaries

The GPUI shell owns document IDs, tabs, focus, navigation history, search scope and user decisions. Disk files, save validation, recovery, search, directory enumeration, CLI parsing and lifecycle remain ordinary Rust modules. Worker completion uses a small renderer-independent callback, connected to a bounded GPUI wake channel. The domain modules are exposed through the library target and can be tested without opening a window. The runtime does not use egui. Retained egui presentation regression tests are test-only; they do not qualify the new interface.

Document reads use two bounded workers, mutations one, and maintenance one. Directory listings use two workers and stream bounded batches. Recursive search and syntax each use a latest-request mailbox. Notifications trigger full document validation; they are not authoritative versions. No periodic scan or repaint loop is intended.

## Editor choice

The editor uses gpui-component 0.5.1 InputState in code-editor mode, retaining one entity per document. Its rope buffer, visible-line rendering, selection, clipboard, undo and GPUI input-method handling provide a smaller implementation surface than writing those primitives again. This is a provisional choice: a rope does not establish large-file performance, long-line responsiveness, IME correctness or accessibility.

The component is vendored under its Apache-2.0 license to add externally supplied highlight spans. The existing bounded syntect/two-face worker remains responsible for syntax. Generation checks reject obsolete results; UTF-16 grammar offsets become UTF-8 byte ranges at the presentation boundary. The other vendored patch corrects xattr 0.2.3's Linux errno name for aarch64 in GPUI's transitive archive dependency. Neither patch changes file-save validation.

The component's default text operations preserve the authoritative buffer. Performance qualification must cover 10 MiB documents, long lines, graphemes, real IME composition and clipboard. None of those requirements is relaxed by choosing this component.

## File safety and services

DiskFile retains observed identity, symlink binding and bytes. Save stages beside the target, preserves supported metadata, syncs, validates and atomically replaces. Hard links, unsupported ownership/metadata and changed destinations stop saving. Atomic replacement is not compare-and-swap against unrelated writers.

Save As rebinds after success; Save Copy retains the original binding. Existing destinations require confirmation of an observed version. Destinations already open in another tab must be closed first. Recovery remains mandatory for normal saves. Failure retains local text and reports the error; the earlier one-attempt recovery bypass is not exposed.

Linux rename uses `renameat2(RENAME_NOREPLACE)` and Trash uses the trash crate's freedesktop implementation. Linux saving copies permissions and extended attributes; failures stop replacement. macOS retains `renamex_np`, NSFileManager Trash and `fcopyfile` metadata preservation behind platform configuration. Linux Reveal opens the containing directory with xdg-open. GPUI supplies native portal file dialogs.

Recovery keeps private, locked session records with exact local text and observed disk bytes. Abandoned records can be claimed from Recovery. Saved history and observed snapshots remain in the domain store. Their full management interface is not yet ported.

## Presentation gaps and qualification

The GPUI interface is an initial port, not feature or performance parity with the macOS prototype. The following remain incomplete:

- Full preferences and editable shortcut controls, native menus, pins/recents/volumes, breadcrumb completion, drops, resizable panes and horizontal navigation gestures.
- Conflict comparison and observed-version overwrite, recovery-history controls and recovery failure alternatives.
- Filename-index controls. Content-result activation rejects changed disk fingerprints and dirty open buffers.
- The earlier bounded Markdown paging, local raster images, relative-link handling and complete dialect parity. The current read view escapes HTML and omits images, including remote images.
- Large-file/long-line performance, real IME, accessibility and macOS behavior. Narrow-window layouts remain incomplete.

The new view virtualizes directory rows and retains per-folder selection and scrolling. Syntax work remains bounded, but extracting full buffer text on edits can be costly. The present Linux evidence is local smoke evidence, not the Mac reference gates or a UI hill-climb campaign.

Closing a requested CLI document currently aborts its request. Use Finish Editing before closing to return success. Save As requires finishing or aborting the request first, rather than offering the earlier abandonment dialog.
