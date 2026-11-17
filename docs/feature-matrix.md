# GPUI feature acceptance

This matrix describes the active GPUI application. It is not release certification. [Glimpse](../glimpse.md) defines scope; [core design](core-design.md) describes the implementation; [qualification](qualification.md) records the distinction between exercised behavior and remaining gates.

| Area | Implemented behavior | Remaining acceptance |
| --- | --- | --- |
| Navigation | One directory list, paths and breadcrumbs, history, Home, volumes, pinned/recent locations, filtering, path completion and document reveal | Complete selection/scroll, gesture, path-transfer and slow-volume tour |
| Documents | Preview and kept tabs, duplicate-name identity, per-document state, source/reading switch and persistent cursor/encoding information | Full preview promotion and switching matrix |
| Search | Folder filtering, scoped filename/saved-content search, exclusions and coverage, pinned/temporary scope, explicit bounded filename snapshot | Full keyboard/scope/error tour, cancellation timing and resource bounds |
| Editor | Rope-backed native input, clipboard, selection, Undo/Redo, Find/Replace/All, indentation, wrapping, line numbers and bounded syntax work | Complete input/selection, supported-size performance and assistive workflows |
| Markdown | Read-only current-buffer rendering, selected CommonMark/GFM structures, bounded local raster images, safe links and literal HTML | Full dialect, overflow, image-error interactions, spoken reading and large-document responsiveness |
| Files | New document/folder, validated Save/As/Copy, no-clobber creation, copy/reveal/external open, single rename and OS Trash | Complete native destination-race, rename/Trash and cancellation matrix |
| Coexistence | Validated disk identity/bytes, clean refresh, explicit conflict decisions and observed-version preservation | Full delayed/concurrent interleavings and device matrix |
| Recovery | Per-session transactional drafts, live-session ownership, automatic abandoned-work recovery, history and storage management | Full interrupted save/cleanup/restore lifecycle |
| Preferences | Appearance, typography, density, scale, editing, search and keymaps; validation and persistence | Every control, shortcut, rendered state and invalid-setting route |
| Accessibility | Semantic controls/text, keyboard menus/decisions, platform input methods and Linux AT-SPI integration | Complete keyboard/screen-reader tour and host compatibility; large-text resource costs remain open |
| CLI | Explicit opens/scope/positions, picker output, waiting requests and deliberate completion/abandonment | Complete byte-safe output, independent callers, picker and destination lifecycle |
| Linux distribution | Compressed ordinary build, verified payload, reversible user installation, desktop entry, icon and MIME registration | Broader installed-session integration |

The interface and renderer use GPUI. Retained historical helper tests and optional vendored components do not establish active-path functionality. macOS remains separately unqualified.
