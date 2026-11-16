# Glimpse

**Product and design source of truth · Core development baseline · Updated 30 October 2026**

Glimpse is a small desktop file navigator and text editor for people whose main work happens in terminals and agent harnesses. Its primary workflow is **find a file, inspect it, make a small edit, and return to the task that brought you there**.

**The application uses a native Rust GUI and renderer. Tauri is excluded.**

This document governs product scope, interaction, visual language, and implementation priorities. Requirements describe the intended product, not a claim that every feature exists. Performance figures are proposed release budgets, not measured results. Engineering choices still requiring evidence are identified explicitly. Change this baseline when a product decision changes; implementation convenience does not silently redefine it.

### Current implementation stage

The macOS development build implements single-list directory navigation, previews, native AppKit source editing, syntax and line numbers, validated Save/As/Copy, no-clobber creation/rename, OS Trash, recovery/history, external-change handling, scoped streaming search, native Markdown reading, functional preferences/keymaps and CLI open/pick/wait. [README.md](README.md) describes actual behavior; [qualification results](docs/qualification.md) separate implemented functionality from release evidence. The 10 MiB long-line case and memory aspiration fail their performance gates. Physical 120 Hz delivery, full VoiceOver and real IME composition remain unverified; no other desktop platform is qualified. These limits do not relax the product targets.

## 1. Purpose and position

Manual editing remains useful when agents do most implementation: adjusting instructions, correcting a paragraph, inspecting configuration, or comparing a generated file with its surroundings. These actions often cross repository and disk boundaries. Glimpse aims to make them immediate with useful installed defaults.

Glimpse makes location, file contents, and the next action immediately available. A project is a convenient folder to return to and search within; it is never a prerequisite for opening a file.

### Intended fit

Glimpse addresses the gap between available tools and the particular way this tool should work. The goal is a deliberate combination of five qualities:

- **Workflow:** immediate file navigation across folders and disks, optional project-like scope, and quick inspection and editing alongside agent work.
- **Compatibility:** a standalone desktop application, useful CLI contracts, conventional platform behavior, and a clean boundary for harness adapters and a later terminal frontend.
- **Performance:** fast startup, responsive traversal and typing, bounded memory, and little background activity.
- **Customization:** a complete installed default with direct control over appearance, shortcuts, typography, density, and editing preferences; executable extensions can follow later.
- **Appearance:** a quiet, precise, cohesive interface whose layout and visual hierarchy suit file navigation and occasional editing.

Originality is not a product requirement. Neovim and GUI Emacs already cover much of this functionality. GUI Emacs includes nonmodal editing and mouse-driven Dired beside a document, built-in CUA bindings, and CLI handoff; Neovim supports external GUIs and rich navigation packages. These are useful references. Glimpse's value lies in the experience it assembles and the control it gives its owner over that experience. [Dired visiting](https://www.gnu.org/s/emacs/manual/html_node/emacs/Dired-Visiting.html), [CUA bindings](https://www.gnu.org/s/emacs/manual/html_node/emacs/CUA-Bindings.html), [Emacs server](https://www.gnu.org/software/emacs/manual/html_node/emacs/Emacs-Server.html), [Neovim UI protocol](https://neovim.io/doc/user/api-ui-events/).

The default should let someone install, open a path, navigate with familiar mouse and keyboard actions, read Markdown, edit, and leave. Configuration improves that experience rather than being required to assemble it. Actual performance, platform compatibility, and low setup burden must be demonstrated during development.

### Existing products and lessons

The following is a comparison of documented capabilities, not a hands-on performance ranking.

| Reference | Substantial overlap | Consequence for Glimpse |
| --- | --- | --- |
| [Ferrite](https://github.com/OlaProeis/Ferrite) | Native Rust/egui, Markdown source and rendering, file tree, quick switching, recursive search, themes, keybindings, and recovery; also broader terminal and structured-data features | Closest feature overlap. Study its navigation and editor components while keeping Glimpse's default scope and presentation deliberate. |
| [GUI Emacs](https://www.gnu.org/software/emacs/manual/html_node/emacs/Dired.html) and [Neovim](https://neovim.io/doc/user/api/), with [Oil](https://github.com/stevearc/oil.nvim), [mini.files](https://github.com/nvim-mini/mini.files), and [Telescope](https://github.com/nvim-telescope/telescope.nvim) | Editing, filesystem navigation, customization, programmatic integration, and extensive package ecosystems | Learn from mature navigation, command composition, and user control without inheriting their entire interaction model. |
| [Sublime Text](https://www.sublimetext.com/) and [Lite XL](https://lite-xl.com/) | Fast editing, navigation, customization, and deliberately lighter interfaces | Useful references for startup, quick switching, transient inspection, and unobtrusive customization. |
| [CotEditor](https://coteditor.com/) | Lightweight native macOS editing, syntax, search, and backup | Platform behavior, international text, and recovery are part of basic quality. |
| [Zed](https://zed.dev/) and [Lapce](https://lap.dev/lapce/) | Rust desktop editors, project navigation, and development integrations | Rust can support this class of application; it does not establish Glimpse's performance. Keep development services outside the core. |
| [Yazi](https://yazi-rs.github.io/docs/quick-start/) and [fzf](https://github.com/junegunn/fzf) | Fast terminal navigation, preview, selection, and composition with other tools | Learn from location history, transient inspection, and explicit selection results. |
| [Fresh](https://github.com/sinelaw/fresh), [Micro](https://micro-editor.github.io/), and [Microsoft Edit](https://github.com/microsoft/edit) | Familiar terminal editing; Fresh also documents an explorer, Markdown preview, and extensive development features | References for a later terminal frontend and for understanding where terminal behavior differs from desktop behavior. |

## 2. Product principles

1. **Reach the file before organizing the work.** Paths, folders, mounted volumes, and recent locations are first-class entry points. Opening a file never creates a project configuration.
2. **Make small actions immediate.** Browsing, typing, and switching documents stay responsive while storage, parsing, or search is busy. Expensive work is explicit, bounded, and cancellable.
3. **Show what an action affects.** Location, recursive search scope, active document, destination, and unsaved changes are distinguishable. A filter or incomplete search never masquerades as an empty folder or a complete result.
4. **Preserve the user's work.** Navigation never discards text. Detected external changes never trigger an ordinary overwrite. Recovery is part of the core.
5. **Use familiar interactions.** Conventional selection, clipboard, shortcuts, menus, and platform window behavior work without setup. Keyboard efficiency and mouse use receive equal attention.
6. **Keep the installed default complete and small.** Navigation, search, editing, Markdown reading, and recovery work offline without plugins, language servers, accounts, or separately installed command-line utilities.
7. **Earn additional scope.** Add a feature when a recurring task demonstrates its value and its cost fits the product. A feature being common in IDEs is insufficient justification.

When principles conflict, preserve document correctness and accessibility first, then responsiveness and clarity, then convenience and visual polish.

## 3. Scope and release boundary

**Core** is the complete first product, although it can be built incrementally. **Extra** is a plausible later addition, off the initial critical path. **External** stays with another application or an explicit adapter. **Excluded** contradicts the current product boundary. An extra is not a roadmap commitment.

| Area | Core | Extra | External or excluded |
| --- | --- | --- | --- |
| Navigation | Single directory list and directory history; direct path entry/completion; mounted volumes; parent/back/forward; pins and recents; local filtering; explicit recursive filename search | Named multi-root collections; richer metadata | Automatic whole-machine indexing and mandatory project setup excluded |
| Text search | Find/replace in one document; scoped content search with file/line results | Project-wide replacement with review; advanced query syntax | Semantic or cloud search excluded |
| Editing | Conventional nonmodal editing; tabs; preview tab; undo/redo; selection/clipboard; indentation; wrap; line navigation; syntax | Multiple cursors; folding; split editors; modal keymaps; additional encodings | LSP, debugger, build system, and refactoring belong outside core |
| Markdown | Authoritative source; read-only rendered view; documented dialect | Simultaneous source/preview; outline; math, diagrams, export | WYSIWYG and executable document content excluded |
| File operations | New text file, new folder, Save, Save As, Save Copy, copy path, reveal/open externally; single-item rename and move to OS Trash | Later move/copy after evidence | Bulk management, cross-volume transfers, archives, and permanent deletion outside core |
| Reliability | External-change handling; save validation; recovery; disconnected/read-only states | Cooperative versioned writes; richer conflict assistance | Silent overwrite and silent lossy conversion excluded |
| Customization | Themes, keybindings, fonts, density, wrap, indentation, search filters | Declarative language/theme packs; selected extension points | Executable plugin runtime, marketplace, and arbitrary UI injection deferred |
| Environments | Desktop GUI and CLI open/pick/wait contracts | Separate TUI; richer local integration protocol; host-specific adapters | Agent runtime, terminal emulator, Git operations, and remote development external |

The product targets macOS, Windows, and Linux. Development and the first vertical slice start on macOS. Cross-platform support is earned through platform acceptance checks; a Mac prototype is not evidence that another platform works. Desktop and CLI form the first release. Terminal editing and visual embedding into a harness are later interfaces.

"Native Rust" means the application UI, editor, and Markdown presentation use a native Rust GUI/rendering stack. **Tauri, Electron, and WebView-based application frontends or document renderers are excluded**, including a Rust/Wasm frontend hosted in a WebView. Platform libraries and non-Rust dependencies remain acceptable where needed; every control need not be an OS widget. Native menus, input, clipboard, accessibility, and window behavior remain quality requirements regardless of rendering toolkit.

## 4. Window and interaction model

Each normal editing session defaults to **one OS window with two resizable panes**: navigator on the left, document area on the right. Separate CLI editing and picker requests may have their own windows. Detaching a pane into a second window is an extra. Either pane can fill the window; the navigator is not permanently constrained to a narrow sidebar. Narrow layouts show one surface at a time with an explicit switch.

The navigator displays one vertically scrollable directory list, starting at Home (`~/`) unless a launch path is supplied. Entering a directory replaces the list in place. Directory history, per-folder selections and scroll positions survive navigation and document editing. Back and Forward traverse visited directories; Parent reaches the containing directory, including beyond the initial folder. The current folder is never an access boundary.

Opening or focusing a document leaves the directory list in place. Favorites occupy a compact menu, with no permanent Favorites rail. Home, mounted volumes and frequent/recent locations remain explicit navigation commands. The document area contains tabs and one source editor or Markdown reading view. Search results appear in a panel anchored to the toolbar search field; they do not replace either main pane.

### Selection and opening

- Clicking a folder enters it in the same list. Clicking a breadcrumb navigates directly to that ancestor. Clicking unused path-bar space or pressing Command-L enters an editable path; Tab completes it and Escape cancels.
- List ↑/↓ selects without opening files. → enters the selected folder or returns to a retained child; ← goes to the parent. Return activates the selection. Toolbar Back/Forward traverse directory history independently of text Undo/Redo.
- Predominantly horizontal gestures emit at most one Back or Forward command. Gesture speed and travel cannot advance multiple locations. The latch lasts through the touch sequence and momentum; unphased wheel bursts reset after an idle boundary. Vertical or strongly diagonal input never changes location. OS natural-scroll settings determine physical direction. Click and keyboard alternatives remain available.
- A file click selects its parent as current and opens a replaceable clean preview with document focus after success. The directory stays a single list. Failed opens leave it available.
- Explicit Open, double-click, or Pin keeps a document tab. Editing a preview pins it immediately. A dirty tab is never replaced by another preview.
- Editor focus and tab switching do not change the directory presentation or location. Opening a search result does not move Location; Reveal in Navigator remains a separate command.
- Switching tabs preserves cursor, scroll, selection, and undo history. Duplicate filenames show enough parent-path context to distinguish them.
- Closing a dirty tab offers Save, Discard, or Cancel. Application exit offers the equivalent decision for dirty documents. Recovery supplements these controls; it does not make intentional discard ambiguous.

Breadcrumb segments navigate directly. Command-L replaces the breadcrumb presentation with inline path entry; it does not add another toolbar. Favorites are created only by explicit pinning. Frequent/recent ranking records successful document-opening visits, coalesces repeated opens within 30 minutes and decays older frequency with a seven-day half-life. Traversal alone does not reorder the list.

Path transfer is core: paste/open an absolute path, complete directory names, copy an absolute path, copy a path relative to the search folder when applicable, and drop files or folders onto the window. Dropping a file opens it; dropping a folder navigates there. Neither operation silently changes a pinned search scope.

## 5. Navigation and search contract

### Location and search scope

**Location** is the current navigation folder: the folder entered most recently, or the parent of a file opened from the navigator. It determines the displayed directory list; it never confines filesystem access. **Search scope** is the folder recursively searched. These are separate state values, not separate application modes.

| Action | Location | Search scope |
| --- | --- | --- |
| Start without an explicit scope | Home folder (`~/`) | Follows Location |
| Enter Folder, Up, Back, Forward, path entry, or Locations | Moves to the requested folder | Follows Location unless pinned |
| Pin Search Scope Here | Unchanged | Pins current Location |
| Go to Search Folder | Moves to pinned folder; available while pinned | Unchanged |
| Unpin Search Scope | Unchanged | Resumes following Location |
| Open a file from the navigator | Its parent folder; the list remains visible | Follows Location unless pinned |
| Switch existing tabs or open a search result | Unchanged | Unchanged |
| Focus the editor | Unchanged | Unchanged |
| Reveal in Navigator | Moves to a containing folder if necessary | Pinned scope remains unchanged |
| Search This Folder | Unchanged | Temporary search uses Location; existing pin remains |

The search heading states the actual target, such as **Find files in glimpse**. The full path is available without guessing from a basename. A pin indicates persistence, not confinement or a permission boundary. Browsing outside the scope is allowed, including on another disk. An out-of-scope document is clearly identified; opening it does not silently enlarge a search.

Effective scope is the temporary search folder, otherwise the pinned folder, otherwise Location. Search This Folder captures Location until the search surface closes or another scope is chosen; navigation does not retarget that temporary search. Pinning or unpinning clears the temporary override. Any effective-scope change cancels old work and clears old results; a new target cannot inherit a Complete label. Reveal changes an unpinned scope when it changes Location. Scope changes do not start a recursive scan until the user requests or updates a search.

Pinning a folder and returning to it provide the initial project-like behavior. No Git repository, project file, workspace import, or automatic root detection is required. Multiple disks can be visited and bookmarked without simultaneous multi-root search.

### Directory browsing

Read only the displayed directory. Stream entries and virtualize visible rows; do not inspect descendants, compute folder sizes, or parse files to draw the directory list. Show dotfiles and ignored entries by default during manual browsing. Optional visibility filters are plainly indicated. Ignore rules govern recursive search rather than whether a user can reach an entry.

Use stable folders-first ordering with predictable name sorting. Incremental enumeration must retain selection by path. Preserve each directory's selection and vertical scroll when leaving and returning. Editing never changes the directory presentation. Loading, empty and failed listings remain visible in the affected list.

Each location has independent loading, stalled, unavailable, permission-denied, and partial states. A slow or disconnected volume cannot block editing or another location. Cancelling abandons its results immediately; some OS filesystem calls may finish later, so worker resources must remain bounded rather than spawning an unlimited thread per timeout. Never automatically mount, authenticate to, or wake every known volume.

Paths remain native OS paths internally. Display escaping must distinguish unusual names without corrupting them. Symlinks are visible; manually opening one is allowed. Recursive search does not follow directory symlinks or cross into nested mounts by default. Explicit overrides show the expanded scope and retain cycle detection.

### Three distinct searches

| Search | Input and target | Result |
| --- | --- | --- |
| Filter this folder | Names already being enumerated in the current listing | Filtered rows; never claims to search descendants |
| Find files | Fuzzy filename/path matching under the displayed recursive scope | Ranked paths with matching portions emphasized |
| Find text | Literal text by default; optional bounded regex under the displayed scope | File, line, and text excerpt; activation opens that location |

Recursive search begins on request and streams results. An explicit Index This Folder action may build a bounded session-only filename snapshot for the chosen scope; indexed queries update as typed. Rebuild and Remove Index remain visible. Snapshots do not claim live or exhaustive coverage; content searches continue to read saved files. There is no automatic all-disk crawl, persistent content index, or background indexing daemon. On-demand scanning still costs I/O: an unvisited disk cannot provide instant exhaustive filename discovery. Navigation speed and cold recursive-search completion are separate measurements.

Recursive searches include dotfiles, respect `.gitignore` and `.ignore`, and exclude version-control metadata, special files, and binary content by default. The exclusions control lists active rules and lets the user deliberately include ignored or metadata paths. An explicitly entered path bypasses discovery filters. Reuse established ignore matching rather than inventing a subtly different glob language. [ripgrep filtering](https://github.com/BurntSushi/ripgrep/blob/master/GUIDE.md), [Rust ignore library](https://docs.rs/ignore/latest/ignore/).

Every search displays **Searching, Complete, Cancelled, Partial, or Failed**, its actual scope, and active exclusions. A result cap, file-size limit, permission failure, disconnected folder, or cancelled scan cannot produce a misleading Complete state. "No matches" means none within the stated coverage. Changing the query cancels previous work; stale results cannot replace newer results or move selection. Ranking favors basename and path-boundary matches with deterministic ties; recency may break ties but never silently hides matches.

Recursive content search reads saved disk contents and labels that source. Command-F searches the current document buffer in a compact floating panel, with replacement disclosed on request. Escape dismisses the panel and returns focus to the editor. No Find controls appear when no document is open. Unsaved drafts are not silently substituted into disk-search results. On activation, revalidate the hit; if a newer disk version or dirty buffer makes its position stale, say so and preserve the buffer rather than pretending the old line still matches.

## 6. Editing and Markdown

### Editing baseline

The initial supported editable format is UTF-8, with or without a BOM. Opening and saving preserves newline sequences, BOM, and final-newline state unless the user explicitly requests conversion. An unchanged open/save round trip is byte-identical. Unsupported encodings and binary files receive a clear read-only explanation and an external-open option; no silent replacement characters, normalization, truncation, or transcoding are permitted.

Core behavior includes keyboard and mouse selection, word/line navigation, undo/redo grouped by logical edit, clipboard, find/replace including replace-all within the active document, go to line, indentation and unindentation, configurable tabs/spaces, soft wrap, line numbers, and line/column status. Selections, deletion, and cursor movement respect grapheme clusters; IME composition and platform shortcuts work correctly. Syntax color never requires a language server.

The initial highlighting set is Markdown, Rust, Python, JavaScript, TypeScript, Go, C/C++, shell, JSON, YAML, TOML, HTML, and CSS. Unknown formats remain fully editable plain text. Choose one maintained highlighting system and bound its work; a parser failure or slow grammar must not block editing. [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) is an incremental parsing candidate, not a commitment or a performance result.

The initial ordinary-editing envelope is files up to **10 MiB**, including fixtures with long lines and non-ASCII text. Larger files enter a clearly labelled, bounded read-only preview or an explicit unsupported state. A partial preview cannot be saved over the full file. Raising limits requires measurements of loading, layout, search, history, and recovery together. Multi-gigabyte editing is not an initial requirement.

New File creates an unsaved document; Save chooses a destination. New Folder and first creation of a named file never replace an existing entry, including one created externally during the operation. **Save As** changes the document's bound destination after success; **Save Copy** writes another destination while leaving that binding unchanged. Existing destinations require explicit replacement handling and the same save checks.

### Markdown contract

Markdown opens as source by default. **Read / Edit** switches the document area between source and read-only rendering of the current buffer, including unsaved text. The renderer does not modify source, take over the source caret, or create a second document. A return to Edit restores the editing position.

Support CommonMark plus explicitly selected tables, task lists, strikethrough, and autolinks. This is a documented dialect, not a claim of pixel or feature parity with GitHub. Code fences use the available syntax colors; unsupported extensions remain readable as source. A CommonMark parser such as [pulldown-cmark](https://docs.rs/pulldown-cmark/latest/pulldown_cmark/) can supply events for native rendering. [CommonMark](https://commonmark.org/), [GFM specification](https://github.github.com/gfm/).

Rendered documents do not execute scripts, code fences, commands, or embedded applications. Raw HTML is displayed as source rather than executed. Local raster images have file and decoded-size limits; remote images are not fetched automatically. Relative links resolve from the document's directory. Text links open through Glimpse when appropriate; external URLs open only after user activation. Diagrams, math, rich embeds, export, and WYSIWYG are extras.

## 7. Coexistence with other writers

Agent work makes external writes common. Correctness must come from document state and save validation, not assumptions about which application owns a folder.

An open document has its own ID, user-facing path, resolved destination, observed filesystem identity, disk base, and monotonically increasing edit generation. These values are distinct. Atomic replacement can change filesystem identity while the pathname stays the same; symlinks and hard links can expose one object through several paths. Never use lossy path strings or a timestamp as the sole identity or version.

| Situation | Required behavior |
| --- | --- |
| Verified disk change; local buffer clean | Refresh after selection/IME activity finishes. Preserve positions in unchanged text; otherwise move to the changed region and indicate relocation. Retain the previous text as one undo step. |
| Verified disk change; local buffer dirty | Preserve the exact local buffer; show a persistent conflict state; ordinary Save is blocked. |
| File deleted, volume removed, or access lost | Keep the buffer and recovery data. Show the actual condition; offer Retry, Save As, or Save Copy. |
| Save fails | Keep the dirty state and destination context. Explain the failure without claiming the file was saved. |
| User edits during an asynchronous save | Mark only the saved generation as persisted. Newer edits remain dirty. |

Verify changes by comparing destination bytes with the disk base. Identical contents only update filesystem identity; they do not clear undo or show a content-change notice. A changed target binding requires a decision before saving. Install a refreshed buffer only after a complete supported read; an oversized, invalid, or partial replacement cannot become an editable partial document. After reload, the new bytes become the disk base; undoing the reload creates a local dirty edit.

Conflict actions are **Compare**, **Keep Editing**, **Reload Disk Version**, **Save Copy**, and a separate deliberate **Overwrite Disk Version**. Reload or overwrite identifies what would be discarded. Comparison is a narrow document operation; it does not require a Git interface or automatic merge engine.

Overwrite authorizes replacement of the specific disk version shown in that decision. Preserve that version and the local candidate, then revalidate. Another observed disk change invalidates the authorization; it never becomes blanket permission to overwrite future versions.

Before replacing an existing file, check its current contents against the disk base. Persist the exact local save candidate and the observed disk version in recovery storage, stage the replacement beside the destination, then re-read and compare the destination's full contents with the base or the specifically authorized overwrite version. Identity and timestamps alone do not satisfy that final check. Use the platform's supported atomic replacement operation; detected divergence stops normal saving. Do not silently fall back to truncate-and-write. Preserve required permissions and supported metadata; if the save strategy cannot do so, fail with alternatives.

**These checks reduce accidental overwrites but cannot exclude simultaneous writes by unrelated applications.** Atomic replacement concerns visibility of a complete file; it is not a portable compare-and-swap against arbitrary writers or a universal power-loss guarantee. A writer can act after the final check. Do not claim that every unseen concurrent version is recoverable. Stronger guarantees require a cooperating writer protocol. Filesystem notifications are hints to revalidate: events may be missed or differ between editors. [notify limitations](https://docs.rs/notify/latest/notify/#known-problems), [Rust rename behavior](https://doc.rust-lang.org/stable/std/fs/fn.rename.html).

Preserve symlinks and save through a validated target binding; show the resolved destination and require a decision if that binding changes. Detect hard-linked files; the initial normal-save policy refuses to break their link relationship and offers Save Copy. Duplicate aliases must not create independent dirty buffers without an explicit warning. External renames are followed only when identity evidence is sufficient; otherwise retain the old binding as missing.

Recovery stores drafts separately from original files, in the OS application-data location with owner-only access where supported. Records have unique session/document ownership and transactional updates. Live instances cannot overwrite, restore, or prune one another's active drafts; startup distinguishes active records from abandoned sessions. Unsaved drafts and unresolved conflicts do not expire automatically. Intentional Discard or Reload Disk Version removes the replaced local draft from automatic restoration; a resolved history copy may remain. Saved-version history evicts resolved copies older than seven days or beyond a 128 MiB history budget; unresolved work is outside that eviction budget. Expose storage usage and deletion controls.

Recovery failure blocks a normal write; offer Retry, Save Copy elsewhere, or an explicit one-attempt Save Without Recovery, unavailable for an unresolved content conflict. The override identifies which copies could not be persisted and bypasses neither destination validation nor metadata/replacement requirements. Records identify exact bytes and generations. Recovery is not a backup service or a promise against storage failure.

Autosave to the original file is off. Background recovery is on. Restart restores recoverable work as a dirty draft and never completes an interrupted staged replacement automatically. Explicit launch arguments determine focus and location, while unrelated recovery remains available without being discarded or automatically imported into an isolated CLI request.

## 8. CLI, harnesses, and terminal use

### Initial CLI contract

The syntax below governs the interface. README identifies the implemented subset and unqualified lifecycle behavior.

```text
glimpse [PATH ...]
glimpse -- PATH ...
glimpse open [--scope DIR] [--line N] [--column N] [--wait] -- FILE ...
glimpse pick [--directory] [--format json|nul] [--scope DIR] [START_DIR]
```

- No path opens the launch working directory; a desktop launch uses the last valid location or Home. Files open pinned tabs. Initial Location is the supplied folder, otherwise the first file's parent, otherwise the caller's working directory. The initial CLI accepts at most one folder per request and rejects additional folders before opening anything. Explicit `--scope` pins search independently. `glimpse -- PATH ...` disambiguates paths named like subcommands.
- Relative paths resolve against the caller's working directory. Positions use one-based lines and Unicode scalar columns, with out-of-range positions clamped visibly and positions inside a grapheme moved to its boundary. `--line`/`--column` apply to the first file; paths are not parsed as `file:line`.
- Missing paths and unreadable targets fail the request with exit 2 and create nothing. `open` accepts files; shorthand also accepts the one initial folder. `--wait` additionally rejects targets outside the supported editing contract. New files are created through New File and Save, not implicitly from a mistyped launch path.
- `--wait` retains original requested destinations separately from document bindings. Closing a requested document or Finish Editing resolves dirty buffers through Save, Discard, or Cancel; Cancel leaves the request waiting. Finish revalidates destinations and resolves missing/conflicted states. Saving alone does not release the caller. Save Copy leaves the original request pending. Save As to another path must explicitly abandon the original destination before rebinding; that request cannot report success. Use Save Copy to keep editing the original.
- The waiting CLI exits 0 when all requested destinations finish successfully, saved or unchanged. Abort, Discard, or Reload Disk Version that abandons requested local edits produces exit 1. An unrecovered invocation, I/O, startup, or application failure produces exit 2; a successful retry can recover a save failure. Exits 1 and 2 do not undo earlier saves. Later external writes are outside this completion observation.
- `pick` selects one existing file, or one directory with `--directory`. Choose explicitly returns the selection; previewing never does. Editing is unavailable. `--scope` pins search only; selection may leave it. Cancel returns 1 and no result.
- Picker output defaults to one JSON object: `version: 1`, `kind: "file"|"directory"`, and `path` containing `encoding`, `base64`, and optional lossless `utf8`. Encode the absolute selected path as `unix-bytes` or `windows-utf16le`; preserve its chosen symlink spelling. `--format nul` returns native path bytes plus a NUL on Unix; on Windows it requires a Unicode path and emits UTF-8, otherwise fails explicitly. Diagnostics use stderr; stdout contains only the result. A picker does not restore or modify the normal editing session.

The initial CLI launches a separate GUI session process for each request. Non-waiting open returns 0 after the GUI has opened the requested targets, or 2 on failure; stdout remains empty and diagnostics use stderr. A waiting CLI remains attached through a private lifecycle channel, then exits with the request's status. The GUI may remain for unrelated documents. No persistent daemon or session reuse is required. Reusing an already-open GUI is an extra and must preserve request ownership and completion semantics.

Harnesses can initially invoke the CLI to open a file at a location, request a blocking edit, or receive a selected path. Copy Path and Reveal in OS cover manual handoffs. A T3-specific integration is a later adapter whose actual host capabilities must be verified. Do not promise that an arbitrary harness can embed a Rust GUI. A future local protocol may expose versioned open/pick/status/completion operations; it does not expose arbitrary evaluation or bypass document conflict rules.

### A standalone application inside a terminal

A full-screen TUI can provide panels, tabs, mouse selection, menus, search, and syntax colors. [Ratatui](https://ratatui.rs/) and [Crossterm](https://docs.rs/crossterm/latest/crossterm/event/index.html) are relevant Rust building blocks; Fresh demonstrates a familiar editor in that environment.

This is a separate terminal frontend, not the desktop GUI rendered unchanged. Character cells, font/layout control, key reporting, clipboard, images, IME, and accessibility depend on the terminal and often on a multiplexer. Enhanced keyboard protocols improve some limitations but are not universal. [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/).

The later TUI should share document, filesystem, search, and save semantics while adapting presentation and shortcuts. It must have usable basic-terminal fallbacks; rich image protocols cannot be a core dependency. A TUI can run where a local desktop cannot, but editing remote-host files over SSH is a separate deployment capability, not something the local GUI gains automatically.

## 9. Design language

The interface should feel like a precise file tool: quiet, readable, dense enough to move quickly, and explicit about location and state. The following rules apply to every future feature.

| Element | Rule |
| --- | --- |
| Hierarchy | File and document content dominate. Location, scope, and active-document identity are the strongest chrome. Secondary actions remain quiet and discoverable. |
| Layout | Two primary surfaces; resizable divider; no required peripheral panels. Overlays return focus to their invoker. Optional features cannot commandeer the default layout. |
| Typography | System UI face for controls and file lists; configurable monospace for source; readable proportional Markdown. Honor font fallback and scaling. Never require an icon font. |
| Spacing | Use a 4 logical-pixel spacing scale. Start with 28-pixel file rows and a 32-pixel toolbar/tab baseline; provide a compact density. These are tunable tokens, not constraints against text scaling or platform requirements. |
| Color | Neutral surfaces, one restrained accent, semantic warning/error/success colors. Selection, keyboard focus, dirty state, and conflict have distinct treatments. Color is never the only signal. |
| Icons | Simple, consistent silhouettes. Folder disclosure is a separate hit target. Important actions have text labels or accessible names and discoverable shortcuts. |
| State | Loading, empty, filtered, incomplete, denied, missing, dirty, and conflicted are distinct. Errors stay near the affected location or document. Save failure and conflict remain visible until resolved. |
| Motion | No decorative animation or artificial delay. Small transitions may clarify a change; respect reduced motion. Background work never animates an idle window continuously. |
| Language | Use concrete verbs: Open, Choose, Save Copy, Pin Search Scope, Reload Disk Version. Prefer folder, location, file, and document. Avoid invented workflow names and claims such as "smart" when a rule can be stated directly. |

All core actions must be available through visible controls or menus and a searchable command palette. The same named commands drive menus, buttons, and keybindings. Use platform conventions: the primary shortcut modifier is Command on macOS and Control elsewhere; preserve standard editing shortcuts. Show bindings in menus and the palette. Escape cancels the active transient operation and restores prior focus; it never discards document text.

Use semantic theme tokens for surfaces, foregrounds, borders, selection, focus, accent, dirty, conflict, and syntax roles. The built-in light and dark themes meet a 4.5:1 minimum for ordinary text, with visible keyboard focus and distinguishable non-text controls. These are design targets informed by W3C guidance, not a claim of application certification. Screen-reader navigation and actual text editing must be tested. [Contrast guidance](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html), [visible focus](https://www.w3.org/WAI/WCAG22/Understanding/focus-visible.html).

Configuration is declarative, versioned, and optional. Defaults always provide a usable application. A small settings surface exposes common preferences; a configuration file permits precise keymaps and tokens. Invalid changes produce a useful error and retain the last valid settings. Do not execute repository-local configuration automatically. Customization must not make the default depend on extensions.

## 10. Technical direction

Keep a small renderer-independent domain layer for document edits/history, file bindings, save/recovery state, search requests, and command definitions. The GUI owns focus, text presentation, and platform interaction. Filesystem work, parsing, and searches run outside input handling with bounded queues, cancellation, and generation-tagged results. Stale results cannot mutate a newer document or search.

This separation supports tests and a possible terminal interface. It is not authorization to build a generalized editor platform, filesystem-provider framework, or custom UI toolkit. Prefer maintained components when their behavior, license, and dependency cost fit the contract. Do not adopt a giant editor subsystem merely to inherit syntax colors, nor write a new text engine before evaluating existing components.

### Begin with modules, not a framework

The initial implementation should use one Cargo package with ordinary Rust modules. Introduce a library target for shared application logic and small executable entry points when the GUI/CLI integration needs them. These are responsibility boundaries, not a requirement to create empty files in advance:

| Boundary | Owns | Must not own |
| --- | --- | --- |
| Document | Text, selections, edit generations, undo, and disk-base state | Widgets, OS file dialogs, or directory traversal |
| Files | Native paths, reads, validated saves, recovery, and change observation | Visual selection or tab layout |
| Search | Scope snapshots, filtering, matching, cancellation, and result coverage | Changing documents or silently choosing a scope |
| Application | Sessions, tabs, commands, settings, and coordination of background results | Rendering details or a second copy of document state |
| UI | Layout, input/focus, text presentation, and accessible controls | Direct save logic or synchronous storage calls during drawing/input |

The domain modules must compile and be testable without opening a window. Prefer concrete types and explicit ownership; use traits when an actual alternative implementation or necessary test boundary requires one. Keep public interfaces small. Extract another crate when a demonstrated dependency, reuse, or build-time need justifies it. A possible future TUI or plugin system alone is not that need. Rust modules already provide encapsulation within a crate. [Rust module privacy](https://doc.rust-lang.org/book/ch07-02-defining-modules-to-control-scope-and-privacy.html).

### Toolkit selection remains an engineering gate

Prototype **egui/eframe and one editor-capable alternative** against the same workload. Floem is a relevant alternative because it exposes a text editor used by Lapce; GPUI and Iced remain candidates if their current APIs and component availability fit. Choose the second candidate after a short component and license assessment, rather than implementing four parallel prototypes. [egui](https://github.com/emilk/egui), [Floem editor](https://lapce.dev/floem/floem/views/text_editor/index.html), [GPUI](https://gpui.rs/), [Iced](https://github.com/iced-rs/iced).

A framework's renderer or text-input demo is insufficient evidence. The spike must demonstrate long-document layout, selection, undo, IME/dead keys, emoji/graphemes, font fallback, native clipboard, screen-reader editing, virtualized tree rows, startup, and idle behavior. Evaluate upstream maintenance and licensing of any editor code separately from the toolkit. Choose a toolkit only after this evidence; if no candidate meets the essentials, revise the implementation strategy before constructing the full application around it.

Extensions begin with stable internal commands and declarative preferences, not a public plugin ABI. Add a runtime only after concrete optional features demonstrate a repeated need. Any later extension system must make activation, capabilities, resource cost, isolation, compatibility, and disablement explicit. Core navigation, editing, and recovery stay available with all extensions disabled.

## 11. Acceptance and development sequence

### Validate the intended experience

Use the owner's actual manual-editing work to evaluate the design. Existing tools provide interaction references and performance baselines; comparison guides implementation rather than deciding whether Glimpse is original enough to build. Record concrete friction, setup effort, visual preferences, and desired customization. Compare a navigation prototype only on interactions it implements; compare editing and recovery when the vertical slice exists. Performance comparisons use equivalent warm/cold conditions and recorded versions/configurations.

Use the same tasks and files:

1. Open a pasted absolute path outside the current repository; edit and save one paragraph.
2. Inspect several similarly named files without accumulating unwanted tabs or losing the starting location.
3. Visit another mounted disk, find a file, then return to the previous folder and document position.
4. Keep one search scope pinned while opening a second location; find a hidden configuration file and explain what was searched.
5. Read Markdown, return to source, and make a small edit.
6. Handle an external replacement while a local edit is dirty; preserve both observed versions and finish deliberately.
7. Choose a path for another process, then complete a blocking CLI editing request.

Record completion, wrong-scope searches, context loss, recovery steps, setup burden, and preference through repeated ordinary use. Refine the interaction until it matches the intended workflow and appearance. Broader user testing can inform a public release; market validation is not a prerequisite for a personally useful tool. Controlled correctness checks remain release obligations.

Reuse maintained libraries or components when they meet the design and reduce maintenance. Evaluate building an editor component against adapting one; the application does not have to become a configuration of another editor. Add features to improve the intended experience, never merely to appear different from existing tools.

### Proposed performance budgets

The current user aspirations are app opening below 100 ms, visually instant folder/file opening, total RAM below 100 MB and CPU below 0.1% during **active use**. Active and idle CPU are separate workloads; the active target must not be reported as an idle target. These supersede the earlier launch/RAM aspirations for current acceptance; the baseline table below remains historical engineering context. Report actual failures, cache conditions and instrumentation limits.

Measure optimized release builds on a named reference machine, OS, display refresh rate, and storage configuration. Establish the initial reference on an ordinary Apple Silicon laptop with local SSD storage; record the actual model and memory before collecting results. Repeat on representative Windows and Linux machines before claiming support. Report p50/p95/p99 where meaningful, sample counts, and cache state.

| Workload | Initial target |
| --- | --- |
| New process, OS caches warm, one local text file up to 1 MiB | p95 first editable frame within 250 ms |
| Launch into an already-running GUI, if reuse is implemented | p95 requested document usable within 150 ms |
| Normal editing within the supported envelope | 120 Hz interaction; 8.33 ms frame-work budget. Physical input-to-present must be measured separately |
| Cached local folder with 10,000 entries | First usable batch within 100 ms; subsequent enumeration does not block input |
| Filename query over 100,000 already-known candidate paths | Updated first results within 100 ms; stale query abandoned within 50 ms |
| Slow filesystem or cold recursive search | Input remains responsive; progress and cancellation remain usable; completion time reported rather than hidden behind a universal latency claim |
| Idle, ten small files open, no search | Aspirational base RSS at most 50 MB excluding document text; report total RSS and define any subtraction. Average idle CPU at most 0.1% of one core over 60 seconds |
| Base desktop distribution | Compressed release package target at most 30 MiB per platform; symbols and optional assets measured separately |

Also measure a genuinely cold launch, content-search throughput, first result versus complete results, save/recovery latency, GPU memory where available, and repeated open/close/search cycles. Search cache, undo, decoded images, and recovery history require explicit bounds. Report missed budgets and their cause; changing a target requires an explicit design decision. Initial development measurements and their limits are documented in README; they do not qualify release budgets.

### Correctness and release gates

The first supported platform must pass keyboard-only and screen-reader file navigation and editing; CJK composition, dead keys, emoji, grapheme selection, and text scaling; byte-preserving UTF-8/BOM/newline round trips; long-line and size-limit behavior; preview-tab promotion; and accurate search coverage states.

Filesystem fixtures must cover same-timestamp content changes, atomic external replacement, deletion, rename, symlinks and target changes, hard links, case behavior, unusual/non-Unicode paths, permission loss, disk-full/recovery failure, and disconnected storage. Exercise edits during saves, concurrent live sessions, abandoned-session recovery against changed disk contents, and saves failing at each stage. Platform-specific metadata and replacement behavior require platform-specific tests. Watcher delivery alone never satisfies the conflict gate.

CLI fixtures must cover spaces and unusual names, caller-relative paths, argument precedence over session restore, no-clobber creation, structured picker output, cancel/failure codes, and multiple waiting requests without releasing unrelated callers. Performance and reliability must be evaluated together; an empty-window demonstration cannot qualify the editor.

### Build in this order

1. **Interaction and technical prototype:** navigation/path transfer, visual hierarchy, toolkit/editor-component assessment, and dependency/license review. Validate the intended feel and identify components to reuse.
2. **Trustworthy vertical slice:** one location, lazy tree, preview/pinned tabs, UTF-8 editing, Save/Save As, exact-generation recovery, and external-change handling.
3. **Complete core:** Locations/history/pins, explicit recursive scope and searches, Markdown reading, syntax set, customization, and CLI open/pick/wait.
4. **Platform qualification:** packaging and platform-specific input, accessibility, filesystem, and performance evidence for macOS, Windows, and Linux.
5. **Optional expansion:** only after core acceptance, select extras from observed demand. Evaluate a TUI and public extension API independently.

### Day-to-day development

The following are development defaults chosen to keep this project understandable and easy to change. They do not expand the product scope.

**Work on one demonstrable user outcome at a time.** The core feature table describes the destination, not a single implementation assignment. A useful slice is "open a path, inspect two files, and return without losing position." "Build the backend" or "finish the UI" is too broad to judge through use. Give each slice a short acceptance scenario and identify the states it must handle: normal, loading, empty, error, and interrupted where applicable.

Use this loop:

1. Sketch uncertain interactions cheaply. If the uncertainty concerns native input, rendering, or performance, test it in a small native Rust prototype.
2. Implement the smallest complete path through the UI, application state, and required file behavior. Label temporary limitations; do not present a prototype as safe for valuable files before save/recovery is verified.
3. Run relevant automated checks, then perform the acceptance scenario in the actual application with mouse and keyboard. A screenshot cannot demonstrate focus, selection, latency, or recovery.
4. Use the build for an ordinary task. Record concrete friction, fix it, and remove superseded implementations before adding adjacent features.
5. Inspect the diff and commit the coherent change once it is understood and verified. Revisit the design document when observed behavior requires a product decision.

UX refinement is expected. Keep visual tokens, typography, focus, and spacing coherent from the first prototype, and refine them while testing real flows. Avoid postponing all appearance work to a final polish pass or treating a changing interaction as grounds for a wholesale rewrite. Iteration should answer observed problems. [Iterative interface design](https://www.nngroup.com/articles/iterative-design/).

**Give features and abstractions a cost.** Before adding either, state the current task it improves, the simpler alternative, and its startup, memory, UI, dependency, and maintenance cost. Defer extras without implementing their infrastructure. Remove abandoned code and dependencies as part of the change that replaces them. Refactor a concrete source of difficulty in a bounded change; avoid mixing a broad architectural rewrite with a new interaction. A new framework, event bus, plugin host, or service is not a substitute for a clear state model.

**Keep verification proportional and continuous.** Once code exists, establish formatting, linting, and focused automated tests in CI. Test document/state transitions and filesystem failure paths directly; use a small number of integration scenarios for complete flows. Avoid brittle screenshot suites and tests that merely repeat implementation details. Reserve separate review for consequential uncertainties such as save correctness, text handling, or a toolkit limitation. Track startup, typing, memory, and package-size budgets as features land. Check platform-sensitive behavior on the other target systems early, even while macOS remains the primary development platform; full platform qualification still occurs before claiming support.

**Keep agent work integrated.** One implementation owner carries a slice through coding, verification, and cleanup. Parallel work should have independent boundaries; unresolved interactions and shared state are poor candidates for simultaneous implementation. Handoffs explain the user-visible result, evidence, limitations, and next step. The owner's use of the running application supplies the judgment about feel and appearance that automated checks cannot supply.

### Git and working records

Glimpse has a dedicated Git repository. Verify its root before Git mutations; never stage or reorganize the enclosing home repository.

Use a runnable main branch and short-lived branches for coherent changes. Keep commits independently understandable: one behavior or bounded refactor with its relevant verification, a short subject, and no unrelated formatting or generated output. Integrate changes through a PR even when merging immediately, so the remote retains a useful change record. Inspect staged contents before committing; stage explicit project paths. Push only when requested. Temporary local work may be incomplete, but distinguish it from a verified milestone. Tag useful working milestones so regressions have a clear comparison point.

Commit application source, manifests, `Cargo.lock`, small fixtures, and useful documentation. Keep build products, caches, downloads, raw logs, recovery files, and large benchmark data outside Git. A tracked application lockfile records exact dependency resolution; it does not replace pinning the toolchain and recording platform prerequisites. Introduce dependencies deliberately and update them separately from unrelated feature work. [Cargo lockfiles](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html), [Git staging and commits](https://git-scm.com/book/en/v2/Git-Basics-Recording-Changes-to-the-Repository).

Publication quality includes every kind of text, not only application code. Keep documentation, comments, snippets, commit messages, PR descriptions, and release notes concise, accurate, and useful to their intended readers. Review incidental text and new files before each commit and authorized push. Do not publish model transcripts, tool output, running commentary, abandoned plans, or unexplained fragments. Keep private working material under ignored `.local/` or outside the repository; `.gitignore` cannot filter prose inside tracked files. Remove private material from unpublished history before pushing, rather than committing it and deleting it later. [AGENTS.md](AGENTS.md) records these contribution rules.

Keep `glimpse.md` as the product/design authority and README as the entry point for actual build/run instructions once they exist. When continuity requires a task record, use one concise private checkpoint for outcome, next action, evidence, and live ownership. Do not turn session transcripts or discarded plans into permanent policy, or create a hierarchy of process documents. Mark superseded decisions clearly so future work does not follow two conflicting designs.

### Development through T3 Code

T3 can host the agent conversation, terminal commands, builds, tests, and Git workflow. Launch the native application on the development machine for desktop interaction checks. The interfaces exposed in this session provide browser previews and iOS/Android device views; arbitrary native desktop-window embedding has not been verified. Plan to use the native window alongside T3 rather than making a browser frontend to fit a preview panel. Mobile simulators do not qualify desktop menus, windowing, keyboard input, or accessibility. A later Glimpse TUI can run in T3's terminal and requires its own terminal-compatibility checks. [T3 preview implementation](https://github.com/pingdotgg/t3code/blob/main/apps/desktop/src/ipc/methods/preview.ts), [T3 devices](https://github.com/pingdotgg/t3code/blob/main/docs/user/devices.md).

## 12. Decisions still requiring evidence

The product boundary above is the baseline. The following are deliberately unresolved engineering decisions, not permission to guess during implementation:

| Decision | Evidence required |
| --- | --- |
| Toolkit and editor-component reuse | Realistic text/accessibility spike, interaction fit, maintenance cost, and license review |
| Text representation and layout/highlighting strategy | Supported-size, long-line, Unicode, history, and latency fixtures; evaluate component reuse before selecting a rope or piece table |
| Per-platform save and metadata strategy | Fault-injection results and documented filesystem limitations; no portable atomicity claims inferred from a function name |
| Search worker/cache limits and larger-file policies | Representative trees, slow mounts, memory measurements, and visible partial-result behavior |
| Exact local integration transport and host embedding | Actual lifecycle needs and supported host APIs; CLI remains the baseline |
| Release packaging, license, and distribution | Dependency compatibility and platform validation; no public name, trademark, or command-name clearance has been established |

The next steps are completing the core contracts and the comparative editor spike, then qualifying native behavior and performance. The design establishes direction; prototypes and measurements establish whether the implementation meets it.
