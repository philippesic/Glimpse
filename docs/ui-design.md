# Glimpse UI design

This document describes the intended native GPUI interface. Current implementation coverage and limitations are recorded in [core design](core-design.md). Requirements are not claims of verified behavior.

Glimpse combines sophisticated appearance with simple functionality and interaction. Its everyday workflow is find a file, inspect it, make a small edit, save, and return to work. Visual refinement should make that workflow feel effortless: well-proportioned controls, readable typography, familiar focus and keyboard behavior, and clear document state. Clean means organized and intentional, with enough context to act confidently.

[glimpse.md](../glimpse.md) governs product scope, navigation and file safety. This document defines how that contract appears on screen; requirements here do not claim that a feature is implemented or qualified. Shared semantic tokens should govern all GPUI surfaces; historical test-only appearance helpers do not establish runtime behavior.

The [native evaluation guide](native-evaluation.md) defines objective criteria, repeatable walkthroughs and evidence-based review loops for implementing this design.

## Simple interaction

Keep the conceptual model small: a location, open documents and an explicit search scope when searching. Opening a file requires no project setup, workspace selection or configuration. Keep one directory list beside the document. Visual refinement must not introduce extra modes, navigation layers, gestures to learn or steps in the normal open/edit/save path.

Give each task an obvious entry point. A command can also appear in the platform menu, context menu and keyboard bindings, but those routes share the same name and behavior. Show frequent actions in context; place occasional actions in an overflow menu. Avoid competing toolbars, repeated action rows and hidden hover-only essentials. Opening a menu should expose useful choices, not start a configuration workflow.

Use progressive disclosure for optional detail. Search scope and incomplete results remain visible because they change the meaning of a result; exclusion-rule editing can be behind a labelled control. A file's name and modified state stay visible; secondary metadata can be disclosed. Recovery works in the background and asks for a decision only when the user must resolve a real condition. Internal generations, queues, caches and storage machinery do not become user-facing concepts.

Simplicity must preserve safety. Routine successful operations are quiet. A failed or destructive operation names the affected file, explains the consequence and presents a clear safe next action, with other relevant choices available nearby. Never hide an unresolved conflict or remove necessary confirmation to make the interface appear simpler.

Judge the experience by the ordinary task: can someone open, locate, edit, save and return without explanation, extra setup or unnecessary decisions? A more polished version should require no additional steps. Retain the established scope; visual sophistication does not justify IDE features, a dashboard, more panes or a larger preference surface.

## Composition and hierarchy

Keep native platform window controls, movement, resizing and fullscreen behavior. Use platform menus and conventional shortcuts. Do not imitate macOS window controls inside the rendered content. The native Rust renderer owns the application surfaces; no WebView is used for the UI or Markdown.

The navigator and document are the two primary surfaces, separated by one resizable divider. Compose the title bar, toolbar, tabs and pane boundaries as one continuous window. Integrate controls with the native window where supported, retaining OS behavior and hit regions. The navigator uses a gently tinted background; the document has a stable, opaque reading canvas. Toolbars and tabs share a coherent chrome treatment.

Use restrained depth in chrome, controls, menus and dialogs. Subtle translucency, tonal gradients, edge highlights and soft shadows may establish material or elevation when they preserve legibility and meet measured performance requirements. Provide an opaque alternative for reduced transparency or insufficient rendering support. Content never needs to shine through source text or file names. Avoid nested cards around ordinary lists, decoration that consumes reading space and permanently empty panels.

Within each surface, establish this reading order:

1. Current folder or document name, active tab and working content.
2. Full location access, search scope, modified/conflict state and immediate actions.
3. Supporting metadata, shortcuts and occasional actions.

Use one strong heading per region. Align labels, icons, fields and baselines. Group related controls with spacing before adding borders. A small labelled action is preferable to an ambiguous icon; a row of equally prominent buttons obscures priority. Keep essential actions visible, with less frequent commands in menus. An empty editor says “Select a file to edit” and can offer Open File; it does not become a dashboard.

## Typography, geometry and density

Use the platform system UI face for controls and file lists, a configurable monospace face for source, and a readable proportional face for Markdown. Register fonts once and retain Unicode fallback. Do not ship an icon font or assume that arbitrary Unicode symbols render consistently.

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
| Canvas | `#FFFFFF` | `#0B0B0C` | Source and reading surface |
| Chrome | `#F6F6F7` | `#111113` | Toolbar, path bar, tabs, search panel and status |
| Sidebar | `#F6F6F7` | `#111113` | Navigator |
| Raised | `#FFFFFF` | `#18181B` | Menus, popovers and dialogs |
| Faint | `#EDEDEF` | `#1F1F23` | Secondary button fill, code blocks and quiet groups |
| Text | `#27272A` | `#F4F4F5` | Primary content |
| Secondary | `#5C5C65` | `#9A9AA3` | Paths, metadata, hints and resting toolbar labels |
| Separator | `#E4E4E7` | `#1F1F23` | Decorative region boundaries |
| Border | `#E4E4E7` | `#2A2A2F` | Outlines of menus, popovers and dialogs |
| Control border | `#8B8B94` | `#6A6A73` | Necessary control boundaries such as text fields |
| Hover | `#EBEBED` | `#1C1C20` | Pointer feedback |
| Selection | `#E2E8F8` | `#1A2747` | Active selection, normal text |
| Inactive selection | `#E9E9EC` | `#222226` | Selection outside the focused pane |
| Accent | `#1D4ED8` | `#5A8BFF` | Focus, links and modified indicator |
| Action fill | `#1D4ED8` | `#2F63E6` | Primary button behind white text |
| Warning | `#9A5800` | `#F5A524` | Partial coverage, recovery concerns |
| Error | `#C81E2B` | `#FF5C63` | Failed actions and conflicts |
| Success | `#0F7A55` | `#34D399` | Confirmed successful outcomes |

The palette follows T3 Code's neutral design language: near-white and near-black surfaces without a colour cast, one blue accent, hairline separators, and depth from soft shadows on floating surfaces rather than outlines on resting controls.

Ordinary text must meet 4.5:1 contrast against its rendered background; necessary control boundaries and focus indicators target at least 3:1. Decorative separators are not sufficient control boundaries. Verify actual compositing, disabled controls, syntax colors and custom themes separately; palette calculations alone do not qualify accessibility. Keep secondary text readable instead of fading essential information.

Keep resting chrome quiet. Use grouping, fill and spacing to establish controls; draw an outline where a boundary is needed, rather than boxing every action. Strong keyboard focus remains explicit. Material highlights and translucency must not weaken text or focus contrast.

Selection is a rounded tinted fill with a leading marker that is accent-coloured while its list has keyboard focus and secondary-coloured otherwise. The active tab carries the same signal on its top edge for the document area. Buttons and fields show a 2-point accent outline when focused. Focus remains visible on selected items and in every theme. Inactive selection retains its shape and normal text with a neutral fill. A modified tab adds a dot and an accessible “Modified” state. A conflict uses an explicit label and actionable notice. Hover, activation, modification and conflict must never collapse into the same signal. Colored text always has a label, shape or other non-color equivalent.

Use a crafted, consistent symbol family with optically balanced strokes, sizes and baseline alignment. Start with 16-point file/folder symbols and disclosure chevrons with separate hit targets. Use platform symbols where supported and permitted, or carefully drawn bundled equivalents. Avoid emoji substitutes, ornamental badges, and icons that require filesystem inspection to paint. Tooltips explain a control and its shortcut; they do not substitute for accessible names.

## Navigation, tabs and focus

Display one directory list at a stable width. Entering a folder replaces that list; opening a file leaves it visible beside the editor. Back/Forward and horizontal gestures share directory history. One gesture emits one step regardless of speed, travel or momentum. Retain per-folder selection and vertical scrolling. Respect Reduce motion and keep vertical scrolling independent.

Toolbar controls share a centered row where space permits. A short window with one pane shows the path bar in Browse and makes the document area available for Source or Reading. Command-L opens inline path entry in either pane. The path bar keeps the current folder visible and offers a scrollable menu for ancestors that do not fit. Visible breadcrumbs navigate directly, and unused path-bar space opens path entry. In this layout, Reading keeps Edit Source beside the tabs and groups Links and image actions together. Keep favorites in a compact menu. Anchor search results to the search field without replacing the navigator or document. Command-F opens Find above Source. The query and Close share a row, options wrap with the pane width, and a short window places secondary options in a menu. Replace has its own input. Resizing preserves the query and replacement fields. Escape returns to editing. Source text and gutter labels share actual layout baselines, and tab stops derive from font metrics and indentation settings.

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

Every command requires a visible/menu route and a useful accessible name. Expose roles, current values, selection, expanded state and modified/conflict state. Provide labelled controls and stable keyboard focus; do not rely on tooltips or painted text for screen-reader semantics. Screen-reader navigation and actual text editing, IME/dead keys, emoji/graphemes and keyboard-only use are separate acceptance checks. Toolkit support does not establish that they work. Respect reduced-motion and increased-contrast preferences when available, and expose a usable manual alternative where platform integration is incomplete.

## Implementation contract

GPUI owns the native application, editor and Markdown surfaces. Retain one editor entity per document for caret, scroll, selection and undo. Apply validated theme and typography preferences together, keeping font fallback and platform scaling. Overlays contain keyboard focus and restore it deliberately when dismissed. Backend editing locks also disable Find/Replace mutations.

Use one toolbar, a tinted navigator and distinguishable document tabs. Primary-modifier-L edits the location. At narrow widths, provide explicit access to navigator and document instead of clipping simultaneous panes. Secondary actions remain reachable through menus. Keep OS-specific dialogs and services behind platform boundaries; no embedded AppKit editor or WebView renderer is part of this design.

Compare and replacement dialogs display read-only excerpts capped at 16 KiB, 160 lines and 240 characters per line. Scrolling these excerpts never lays out a full large document. No full document clone belongs in an idle dialog redraw. Reload/overwrite decisions capture the observed version and reject a changed local generation or disk snapshot before submitting the existing validated write path.

## Performance and acceptance

Appearance adds no background worker, index, periodic timer or unconditional repaint. Install themes and load any required fonts/assets at startup or preference changes, not every frame. Resolve colors cheaply. Paint visible rows and retain stable widget IDs. Directory enumeration and sorting remain outside painting. The current navigator still builds a filtered row list from its bounded directory cache; include that cost in large-list measurements. Keep filesystem work outside input handling. Use static pending labels rather than idle spinners. The default caret is steady.

Brief, event-driven transitions may clarify hover, selection, opening a popover or changing presentation. They must be interruptible, never delay input or focus, and stop requesting frames when settled. Respect reduced motion. Avoid decorative looping, exaggerated bounce and mandatory animated scrolling. Profile material effects and transitions in the actual native release build; neither their affordability nor their cost follows from a screenshot. Opaque surfaces and static controls are sufficient. Motion must respect Reduce motion and never delay commands.

The current user aspirations are startup under 100 ms, folder and file opening within 100 ms, total RAM below 100 MB and idle CPU at most 0.1% of one core, with active CPU proportional to the interaction. Measure active and idle CPU separately. The earlier baseline additionally asks for 120 Hz interaction. These are aspirations, not measured results; [the product baseline](../glimpse.md#proposed-performance-budgets) retains its separately stated proposed budgets. At 120 Hz one refresh interval is about 8.3 ms; meeting a frame-work budget alone does not establish input-to-present latency. Measure native release builds on a named machine/display, with explicit warm/cold startup, sample counts and idle intervals. Report total RSS alongside any explicitly defined text-memory exclusion, rather than silently subtracting an estimate. Styling does not qualify performance.

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
