# Glimpse UI design

Glimpse combines sophisticated appearance with simple functionality and interaction. Its everyday workflow is find a file, inspect it, make a small edit, save, and return to work. Visual refinement should make that workflow feel effortless: well-proportioned controls, readable typography, familiar focus and keyboard behavior, and clear document state. Clean means organized and intentional, with enough context to act confidently.

[glimpse.md](../glimpse.md) governs product scope, navigation and file safety. This document defines how that contract appears on screen; requirements here do not claim that a feature is implemented or qualified. [src/appearance.rs](../src/appearance.rs) holds the built-in appearance tokens.

The [native evaluation guide](native-evaluation.md) assigns the Mac reference apps and defines repeatable walkthroughs and comparison/review loops for implementing this design.

## Simple interaction

Keep the conceptual model small: a location, open documents and an explicit search scope when searching. Opening a file requires no project setup, workspace selection or configuration. Keep one directory list beside the document. Visual refinement must not introduce extra modes, navigation layers, gestures to learn or steps in the normal open/edit/save path.

Give each task an obvious entry point. A command can also appear in the platform menu, context menu and keyboard bindings, but those routes share the same name and behavior. Show frequent actions in context; place occasional actions in an overflow menu. Avoid competing toolbars, repeated action rows and hidden hover-only essentials. Opening a menu should expose useful choices, not start a configuration workflow.

Use progressive disclosure for optional detail. Search scope and incomplete results remain visible because they change the meaning of a result; exclusion-rule editing can be behind a labelled control. A file's name and modified state stay visible; secondary metadata can be disclosed. Recovery works in the background and asks for a decision only when the user must resolve a real condition. Internal generations, queues, caches and storage machinery do not become user-facing concepts.

Simplicity must preserve safety. Routine successful operations are quiet. A failed or destructive operation names the affected file, explains the consequence and presents a clear safe next action, with other relevant choices available nearby. Never hide an unresolved conflict or remove necessary confirmation to make the interface appear simpler.

Judge the experience by the ordinary task: can someone open, locate, edit, save and return without explanation, extra setup or unnecessary decisions? A more polished version should require no additional steps. Retain the established scope; visual sophistication does not justify IDE features, a dashboard, more panes or a larger preference surface.

## Composition and hierarchy

Keep the native title bar, traffic lights, window movement, resizing and fullscreen behavior. Use platform menus and conventional shortcuts. Do not imitate macOS window controls inside the rendered content. The native Rust renderer owns the application surfaces; no WebView is used for the UI or Markdown.

The navigator and document are the two primary surfaces, separated by one resizable divider. Compose the title bar, toolbar, tabs and pane boundaries as one continuous window. Integrate controls with the native window where supported, retaining OS behavior and hit regions. The navigator uses a gently tinted background; the document has a stable, opaque reading canvas. Toolbars and tabs share a coherent chrome treatment.

Use restrained depth in chrome, controls, menus and dialogs. Subtle translucency, tonal gradients, edge highlights and soft shadows may establish material or elevation when they preserve legibility and meet measured performance requirements. Provide an opaque alternative for reduced transparency or insufficient rendering support. Content never needs to shine through source text or file names. Avoid nested cards around ordinary lists, decoration that consumes reading space and permanently empty panels.

Use [Things' OS 26 refresh](https://culturedcode.com/things/blog/2025/09/things-for-os-26/) and [Bear's Tahoe refinement](https://community.bear.app/t/bear-update-os26-ready/18219) as finish references: typography, coherent symbols, gentle material depth and carefully shaped selection. Use [Finder](https://support.apple.com/en-gb/guide/mac-help/mchle9f0a1b2/27/mac/27) and [Nova](https://help.nova.app/welcome/quick-tour/) for window composition and useful file/editor density. Borrow their craft within Glimpse's existing workflow and scope.

Within each surface, establish this reading order:

1. Current folder or document name, active tab and working content.
2. Full location access, search scope, modified/conflict state and immediate actions.
3. Supporting metadata, shortcuts and occasional actions.

Use one strong heading per region. Align labels, icons, fields and baselines. Group related controls with spacing before adding borders. A small labelled action is preferable to an ambiguous icon; a row of equally prominent buttons obscures priority. Keep essential actions visible, with less frequent commands in menus. An empty editor says “Select a file to edit” and can offer Open File; it does not become a dashboard.

## Typography, geometry and density

Use the macOS system UI face for controls and file lists, a configurable monospace face for source, and a readable proportional face for Markdown. Register fonts once and retain Unicode fallback. The eframe `system_fonts` feature supplies fallback; it does not by itself select the primary system UI face. Do not ship an icon font or assume that arbitrary Unicode symbols render consistently.

| Token | Comfortable default | Compact |
| --- | --- | --- |
| UI/file-list text | 15 pt | 15 pt |
| Source text | 14.5 pt, minimum line height ×1.4 | Same |
| Supporting text | 12 pt | 12 pt |
| Section heading | 18 pt, explicit bold face | 18 pt, explicit bold face |
| File row pitch, including spacing | 32 pt | 24 pt |
| Toolbar/tab control baseline | 36 pt | 28 pt |
| Standard control minimum height | 24 pt | 20 pt |
| Pane inset | 12 pt | 8 pt |
| Native source inset | 8 pt horizontal, 6 pt vertical | Same |

These are logical-point baselines, not maximums or a finished component specification. Enlarge rows and controls to fit actual text and hit areas; the main toolbar may need more height than the compact tab strip. Compact mode reduces space, not font legibility. Use a 4-point spacing scale, typically 4 within a group, 8 between controls, 12–16 between groups, and 20–24 around a reading surface or settings section.

Tune typography by role: clear document and section headings, confident action labels, readable file names and subordinate metadata. Verify actual font selection, weight, line height and optical alignment in the native app. Give toolbar groups, fields, tabs, rows and popovers their own coherent geometry rather than applying one rounded rectangle everywhere. Built-in corners use 7 points for controls, 9 for menus and 12 for dialogs. Larger surfaces can use softer curvature; pane boundaries stay precise. Align nested curves with their insets.

Source text uses a stable line height and an aligned, secondary line-number gutter when available. Keep source selectable and avoid decoration behind every line. Markdown uses a comfortable line measure, explicit heading levels, distinguishable links, readable code blocks and table overflow. Read/Edit is a labelled control; returning to Edit restores the source position.

At larger text sizes, compute row pitch from the resolved font height plus padding, and use the same pitch for painting, virtual row ranges and scroll-to-selection. A fixed-height viewport with larger painted text is incorrect. Zoom, font preferences and OS display scaling must not be applied twice.

## Color and interaction state

Default to System appearance with explicit Light and Dark choices. Every surface, popup and inline notice follows the active theme. Use neutral surfaces, one blue action accent and semantic warning, error and success foregrounds. Restrained icon color can help distinguish familiar destinations and file types. The built-in opaque palette provides the foundation and fallback for material treatments:

| Role | Light | Dark | Use |
| --- | --- | --- | --- |
| Canvas | `#FFFFFF` | `#15181E` | Source and reading surface |
| Chrome | `#F2F4F7` | `#1D2027` | Toolbar, tabs and status |
| Sidebar | `#F1F6FA` | `#1C2027` | Navigator |
| Raised | `#FFFFFF` | `#252A33` | Controls and dialogs |
| Faint | `#E6E8EC` | `#343B45` | Code blocks and quiet groups |
| Text | `#17213A` | `#F1F3F5` | Primary content |
| Secondary | `#647085` | `#B5BEC9` | Paths, metadata and hints |
| Separator | `#E1E6ED` | `#343B45` | Decorative region boundaries |
| Control border | `#CBD3DF` | `#8B96A5` | Necessary control boundaries |
| Hover | `#E1E7EE` | `#39424F` | Pointer feedback |
| Selection | `#DCEBFC` | `#243E60` | Active selection, normal text |
| Inactive selection | `#E5EBF2` | `#3A424D` | Selection outside the focused pane |
| Accent | `#007AFF` | `#409CFF` | Focus, links and modified indicator |
| Warning | `#805600` | `#EBC378` | Partial coverage, recovery concerns |
| Error | `#B42332` | `#FF9EAA` | Failed actions and conflicts |
| Success | `#256B3D` | `#89D6A3` | Confirmed successful outcomes |

Ordinary text must meet 4.5:1 contrast against its rendered background; necessary control boundaries and focus indicators target at least 3:1. Decorative separators are not sufficient control boundaries. Verify actual compositing, disabled controls, syntax colors and custom themes separately; palette calculations alone do not qualify accessibility. Keep secondary text readable instead of fading essential information.

Keep resting chrome quiet. Use grouping, fill and spacing to establish controls; draw an outline where a boundary is needed, rather than boxing every action. Strong keyboard focus remains explicit. Material highlights and translucency must not weaken text or focus contrast.

Selection is a tinted fill; keyboard focus adds a 2-point accent outline. Focus remains visible on selected items and in every theme. Inactive selection retains its shape and normal text with a neutral fill. A modified tab adds a dot and an accessible “Modified” state. A conflict uses an explicit label and actionable notice. Hover, activation, modification and conflict must never collapse into the same signal. Colored text always has a label, shape or other non-color equivalent.

Use a crafted, consistent symbol family with optically balanced strokes, sizes and baseline alignment. Start with 16-point file/folder symbols and disclosure chevrons with separate hit targets. Use platform symbols where supported and permitted, or carefully drawn bundled equivalents. Avoid emoji substitutes, ornamental badges, and icons that require filesystem inspection to paint. Tooltips explain a control and its shortcut; they do not substitute for accessible names.

## Navigation, tabs and focus

Display one directory list at a stable width. Entering a folder replaces that list; opening a file leaves it visible beside the editor. Back/Forward and horizontal gestures share directory history. One gesture emits one step regardless of speed, travel or momentum. Retain per-folder selection and vertical scrolling. Respect Reduce motion and keep vertical scrolling independent.

Toolbar controls share one fixed-height centered row. Breadcrumb names navigate immediately; unused path-bar space and Command-L enter a path. Keep favorites in a compact menu. Anchor search results to the search field without replacing the navigator or document. Command-F opens a small floating panel with optional Replace; Escape returns to editing. Source text and gutter labels share actual layout baselines, and tab stops derive from font metrics and indentation settings.

Show the location basename prominently and make the full native path available through path entry, copy and disclosure. Truncate names only for display; preserve extensions and sufficient parent context for disambiguation. Long names, unusual paths and symlink indicators must not push actions offscreen. Never derive path identity from display text.

Tabs form one continuous strip. Each tab groups its file label, state and close control; the close button has a filename-specific accessible name. Active tabs visually connect to the document canvas. A preview uses italic text and an accessible Preview state, while editing pins it immediately. Modified tabs retain a dot independently of hover. Duplicate basenames include parent context. Overflow scrolls and keeps the active tab reachable; it does not shrink labels to illegibility. Tab switching preserves selection, caret, scroll and undo.

Keep a predictable keyboard path among navigation controls, directory list, tabs and editor. Arrow keys move within composite controls; Tab moves between controls. Custom-painted rows still need semantic selection and focus. Return activates; Escape cancels a transient operation and restores the invoker's focus. A dialog contains keyboard focus, and cancelling it returns focus without changing the document. Do not interpret an unrelated toolbar click as editor focus or interfere with IME composition.

## Search and status

Keep three clearly named operations: Filter This Folder, Find Files and Find Text. A folder filter acts on the current listing; recursive search has an explicit scope heading and full-path access. Show whether scope follows Location, is pinned or is temporarily overridden. Out-of-scope documents are labelled without enlarging the search. Opening a search result does not silently move Location; Reveal in Navigator is separate.

Expose the searches through a compact, contextual surface, with a clear operation choice when needed. Do not permanently stack three search boxes. Keep advanced filters behind a labelled control while showing the effective scope and exclusions summary beside results. Document Find belongs to the editor and keeps its familiar shortcut.

Search results state scope, exclusions, count and coverage: Searching, Complete, Cancelled, Partial or Failed. A result limit or denied branch makes coverage partial. “No matches” includes the coverage context. Find Text labels saved disk contents as its source; document Find searches the current buffer. Revalidate stale results without overwriting a dirty buffer. Keep query focus stable as batches arrive; provide Cancel while searching. A static progress label with worker-driven updates is sufficient.

Use status near the affected item: folder errors in the navigator, save state beside the document, search coverage with results. A compact document footer can show encoding, line endings, caret position and read-only state when those values are known. Do not invent metadata or fill the footer with permanent help text. Save success need not interrupt typing; failure remains visible and the tab remains modified. Dismissing an explanation must not erase an unresolved conflict indicator.

## Settings

Use a small settings surface with labelled controls and useful installed defaults. Show common preferences first and disclose detailed controls within their relevant group. Categories organize available settings; they do not require six permanent sections or empty placeholders. Use a category list only when the amount of content warrants it. Keep labels aligned, help beside the relevant setting, and Reset scoped to the current group. Edit preferences in a draft and apply validated changes together. Invalid font names or shortcuts leave the live configuration untouched. Preserve document state while applying appearance changes. Closing settings restores focus; it does not discard document edits.

| Category | Intended controls |
| --- | --- |
| General | Startup/location behavior and navigation preferences |
| Appearance | System/Light/Dark, comfortable/compact density, UI typography and text scaling |
| Editor | Source font/size, wrap, indentation and line numbers |
| Search | Visible default scope behavior and recursive exclusions |
| Shortcuts | Searchable named commands, current bindings and conflicts |
| Recovery | Actual draft storage/status, storage usage and deliberate cleanup |

An unavailable feature is omitted or explained honestly; a switch must not imply a safety capability that does not exist. Recovery cleanup distinguishes unresolved drafts from resolved history and cannot silently delete work. Avoid broad “Reset everything” actions. Repository-local configuration is never executed automatically.

## File dialogs, conflicts and recovery

Use native file and folder dialogs with accurate titles, starting locations and selection types. Open File and Go to Folder are distinct commands; Command-O means Open on macOS. Save As chooses a destination and rebinds only after success. Save Copy writes elsewhere while retaining the original binding. Cancellation leaves text, location, binding and focus intact. Native replacement confirmation does not replace Glimpse's destination validation.

A close/quit decision identifies the document and provides Save, Discard and Cancel. Save is the primary action when safe; Cancel is reachable with Escape; Return never triggers Discard or overwrite. Disable repeated saves while a save is pending, retain visible progress, and keep newer edits modified. Errors remain actionable instead of closing the decision as if saving succeeded.

Conflict UI identifies the path, local modifications and detected disk change. Offer Compare, Keep Editing, Reload Disk Version and Save Copy according to implemented capability. A separate Overwrite Disk Version decision names what will be lost and applies only to the observed version. Never show a destructive option that the file layer cannot safely execute. Recovery failure offers Retry and Save Copy; any explicit Save Without Recovery follows the product's one-attempt restrictions. Missing files, read-only destinations and disconnected volumes keep their buffers and explain the actual condition.

Startup recovery identifies recoverable documents and restores drafts as modified, without silently writing originals or treating another live session's work as abandoned. Safety copy and recovery state must be grounded in successful persistence; reassuring language cannot substitute for it.

## Scalable layout and accessibility

At normal widths keep both primary surfaces usable, with a draggable divider and preserved user sizing. At narrow widths show one primary surface at a time with an explicit Navigator/Document switch. Move secondary actions into an overflow menu before clipping primary controls. Search inputs and dialog text wrap; settings can replace the category column with a category picker. A large display increases useful content area, not control size or empty chrome. Reading text keeps a comfortable line measure while source and tables can use the available width.

Qualify the minimum window size, a wide window, long names, both densities, both themes, large text and multiple display scales. Menus, dialogs, footer state and tab close controls must remain reachable. Settings and errors cannot rely on a fixed dialog width that truncates their actions.

Every command requires a visible/menu route and a useful accessible name. Expose roles, current values, selection, expanded state and modified/conflict state. Provide labelled controls and stable keyboard focus; do not rely on tooltips or painted text for screen-reader semantics. VoiceOver navigation and actual text editing, IME/dead keys, emoji/graphemes and keyboard-only use are separate acceptance checks. Enabling AccessKit does not establish that they work. Respect reduced-motion and increased-contrast preferences when available, and expose a usable manual alternative where platform integration is incomplete.

## Implementation contract

`appearance::install(ctx, Density)` registers the installed SF UI/mono faces once per context and configures both themes. Keep Unicode fallback. Reinstall styles only at setup or density changes; `ctx.set_theme` and `ctx.set_zoom_factor` apply validated preferences. `palette(ui)` resolves semantic colors; `Metrics::for_density` supplies point baselines. Shared symbol buttons expose accessible names and focus rings.

The window uses a transparent native title bar with OS traffic lights, one toolbar, a tinted navigator and grouped document tabs. Empty path chrome provides window dragging. Breadcrumb segments open folder/file menus; Command-L edits the path in place. The toolbar search field is editable, with results in the navigator. Document Find is a dismissible card with a replacement disclosure; AppKit reserves the card’s measured height so its child view cannot cover the Rust controls. Below 760 logical points, explicit Navigator/Document or Navigator/Preview choices replace simultaneous panes. Secondary toolbar actions move into More.

`Editors::begin` and `finish` bracket exactly one presentation pass. Retain native views for caret, scroll and undo. Hide the AppKit source whenever a Rust modal or overlapping menu is open; give forms host focus and restore the relevant source deliberately. Backend editing locks also disable Rust Find/Replace mutations. Native source geometry uses the current zoom exactly once.

Compare and replacement dialogs display read-only excerpts capped at 16 KiB, 160 lines and 240 characters per line. Scrolling these excerpts never lays out a full large document. No full document clone belongs in an idle dialog redraw. Reload/overwrite decisions capture the observed version and reject a changed local generation or disk snapshot before submitting the existing validated write path.

## Performance and acceptance

Appearance adds no background worker, index, periodic timer or unconditional repaint. Install themes and load any required fonts/assets at startup or preference changes, not every frame. Resolve colors cheaply. Paint visible rows and retain stable widget IDs. Directory enumeration and sorting remain outside painting. The current navigator still builds a filtered row list from its bounded directory cache; include that cost in large-list measurements. Keep filesystem work outside input handling. Use static pending labels rather than idle spinners. The default caret is steady.

Brief, event-driven transitions may clarify hover, selection, opening a popover or changing presentation. They must be interruptible, never delay input or focus, and stop requesting frames when settled. Respect reduced motion. Avoid decorative looping, exaggerated bounce and mandatory animated scrolling. Profile material effects and transitions in the actual native release build; neither their affordability nor their cost follows from a screenshot. The built-in presentation uses opaque surfaces and static controls, with dialog shadows for elevation. It adds no blur or translucency. Control-state transitions settle within 100 ms; column snapping uses 180 ms. Both respect Reduce motion and do not delay commands.

The current user aspirations are startup under 100 ms, visually instant folder/file opening, total RAM below 100 MB, and CPU below 0.1% during active use. Measure active and idle CPU separately. The earlier baseline additionally asks for 120 Hz interaction. These are aspirations, not measured results; [the product baseline](../glimpse.md#proposed-performance-budgets) retains its separately stated proposed budgets. At 120 Hz one refresh interval is about 8.3 ms; meeting a frame-work budget alone does not establish input-to-present latency. Measure native release builds on a named machine/display, with explicit warm/cold startup, sample counts and idle intervals. Report total RSS alongside any explicitly defined text-memory exclusion, rather than silently subtracting an estimate. Styling does not qualify performance.

Accept a shipped surface only after exercising its normal, loading, empty, failed and interrupted states where applicable. The integrated app must demonstrate:

- Directory traversal → preview → edit → return with selection and scroll retained, using keyboard and pointer.
- The normal open/edit/save/return path requires no new setup, modes or decisions after the appearance changes; optional detail stays out of the way until relevant.
- Readable light/dark surfaces, coherent tabs, visible focus, usable density/text scaling and no clipped actions at minimum size.
- Typography, symbols, control geometry and window composition are deliberately resolved across all shipped surfaces; default toolkit styling alone does not meet the visual finish target.
- Search scope and coverage that match actual work, settings that apply only functional preferences, and cancellation that restores focus.
- Dirty close/quit, failed save, external conflict and recovery states that preserve text and accurately describe the available actions.
- Native keyboard and accessibility checks plus measured interaction, startup and idle behavior, with unsupported or untested cases stated explicitly.

A screenshot demonstrates appearance at one instant. It cannot establish focus restoration, file safety, accessibility or performance.

## Disk-wide navigation and explicit indexing

Favorites are explicit file or folder pins in a compact directory-header menu. Home, Volumes and frequent/recent destinations remain in the folder menu. Repeated document opens within 30 minutes count as one visit; scores decay with a seven-day half-life. Traversal does not add visits. The navigator displays one directory at a stable width, retaining selection and vertical scroll for return visits.

Index This Folder builds one bounded filename snapshot only after a click. It replaces the prior snapshot, lives only for this process, respects search exclusions and never indexes content. Search labels its snapshot coverage and offers Rebuild and Remove Index. Directory and document observation remain separate from snapshot freshness.

## Reference composition refinement

The toolbar groups the current path in a shaped control and keeps search beside it; narrow windows use compact search and overflow controls. Folder filtering opens on request. A single navigation header replaces stacked folder/location rows, and document tabs share the canvas edge without a duplicate absolute-path strip. Folder and file-type symbols provide distinct color/shape cues.

Preferences use General, Appearance, Editor, Search, Shortcuts and Recovery categories, a wide-window category sidebar, grouped controls and functional theme previews. Narrow windows use a category menu. Search results emphasize the file, then its parent path, then a highlighted saved-content excerpt and line number. Arrow keys select results and Return activates them. The filename snapshot controls remain optional and scoped; freshness is visible.

The source editor paints a quiet current-line background without modifying text attributes or creating undo steps. The setting can be disabled. Cmd-L selects the complete path for replacement. Read mode returns to source when activating a code tab. These changes preserve the existing file-safety, recovery and explicit-action boundaries; they do not certify pixel parity with the supplied references.

## Reference geometry in the macOS implementation

The default window is 1280 × 844 logical points. Its header aligns the native traffic-light region, navigator toggle, directory Back/Forward, clickable breadcrumbs and editable search on one centered control row. Tabs remain about 42 points. The single directory pane defaults to 280 points and retains the user's divider width across navigation and editing. Search and Find use compact native child windows so the AppKit source stays visible. The source gutter is measured from number glyphs, with source-font-derived tab stops and zoom-scaled insets. Gutter labels use actual source glyph baselines, including configured paragraph spacing.

The toolbar uses a left-aligned shaped breadcrumb group, editable search, and compact navigation actions. New File lives beside document tabs; named-file and folder creation live in the navigator header. The permanent folder filter is disclosed from the folder menu. Tabs connect to the document canvas; folder and file-type symbols use layered vector shapes. Source uses explicit light/dark syntax colors and an italic comment trait, with the current-line tint continued through the gutter. Native paragraph layout supplies the configured minimum line height; font and spacing changes update attributes without replacing document characters. Saved font sizes remain unchanged when defaults change. Narrow document mode has one toolbar and one tab strip, with a toolbar navigator toggle.

Appearance settings group real theme previews, density, reading controls and everyday editor switches, including font size and line height. Wide settings uses 16-point body text and 17-point category labels with 44-point rows. The full Editor category retains font, indentation and reset controls. Reading mode styles standalone plain headings on small pages; nested/linked headings and pages with reference definitions remain with the CommonMark renderer to preserve semantics. Larger pages retain viewport rendering.

The implementation uses opaque colors rather than the reference's translucent macOS material. Settings is an in-app modal rather than a separate preferences window. Conflict comparison aligns bounded read-only line excerpts, using red local-only and green disk-only rows. It never acts as a merge or save input. CommonMark's code-block copy glyph and complex-page typography also retain library styling. Unimplemented reference controls, including tags and additional appearance options, are omitted.

Search match text uses a yellow fill distinct from selection: light `#FFE878` with `#3B3100` text, dark `#756018` with `#FFF4CA` text. Both pairs pass a 4.5:1 text contrast check.

Conflict alignment reads at most the first 16 KiB and 160 lines per version, showing at most 240 characters per line. A bounded line alignment uses at most 161² cells; results are cached for the local generation and bounded disk prefix. Clipped lines are marked as uncompared, truncation is labelled, and final-newline differences remain explicit. Opposite-side gaps preserve row height. Full original snapshots and destructive resolution validation are independent of this presentation.
