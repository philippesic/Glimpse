# macOS qualification

Results from the 28 October 2026 development build. These are measured development results, not release certification. Tests ran on a Mac15,10 / M3 Max, 36 GiB RAM, macOS 27.0, with a reported 120 Hz display and Retina backing. Other user applications were not controlled. Windows and Linux were not tested.

## Functional evidence

The current suite has 83 passing tests and two separately runnable CPU benchmarks (navigation and indexed filenames). Strict all-target clippy, formatting and an ad hoc signed release package pass. Tests cover byte/metadata preservation, no-clobber create/rename, OS Trash, destination races and alias reservations, recovery/import/history/live-session ownership and failures, scoped search cancellation and limits, preferences and CLI lifecycle state.

Actual native interaction additionally verified:

- File/folder creation, single-file rename and OS Trash; exact UTF-8 source Save and Save Copy.
- Save As into an open clean destination, an external change after confirmation, preservation on failure, and a successful retry that reconciles the destination tab.
- External clean refresh with Undo, dirty-save conflict, explicit Reload and independent observed-disk snapshot restoration.
- A persisted Unicode draft after controlled process termination, one-time recovery import and Save As resolution without recurrence.
- Picker file JSON, directory NUL output and Cancel, with exact output and exit status; read-only preview rejects paste.
- CLI aliases sharing a buffer; Save leaves a waiting caller pending; Finish completes requested destinations while unrelated tabs remain. Cancel, Discard and Save As abandonment were exercised.
- Search-hit line activation without moving Location; filtering, preview selection and scrolling in an actual 10,000-entry directory with virtualized rows; native clipboard path round trip; Markdown reading; tab/find/settings focus, dirty Quit/Cancel and Quit/Discard.
- Japanese, combining accents and emoji retained exactly by paste/save/recovery; acute dead-key input; family-emoji Backspace and Undo as one grapheme.

A reported picker Choose/exit stall did not reproduce in the controlled successful picker sessions. Terminal cancellation state now rejects late validation results. Testing the real CLI wrapper exposed and repaired a Darwin accepted-socket nonblocking readiness failure. A later delayed Quit dialog prompted an explicit repaint repair; the rebuilt visible Quit/Cancel/Discard path passed. Intermittent automation `noWindowsAvailable` failures remain part of the environment evidence, rather than being used to dismiss lifecycle defects.

## Performance

Targets remain aspirational 50 MB idle RAM excluding text, at most 0.1% idle CPU, under 250 ms to a usable app frame, and physical 120 Hz file-view interaction (8.33 ms). Memory below is total measured memory; no guessed text subtraction was made. MiB uses 1,048,576 bytes.

| Scenario | Result | Qualification |
| --- | --- | --- |
| Final tiny plain file, five warm launches | GUI marker 213.1–230.3 ms; full CLI acknowledgement 235.2–292.4 ms | GUI markers meet 250 ms; one full CLI acknowledgement misses; cold launches and physical first-present are unmeasured |
| Earlier three tiny documents | GUI marker 252.6 ms | Misses 250 ms in this sample |
| Final tiny plain file, 60.005 s after readiness | CPU 0.0483% of one core; RSS 155.58 MiB; physical footprint 137.44 MiB | CPU meets this interval; RAM misses, even with negligible text |
| Earlier three-document UI session, 15.002 s | CPU 0.00168%; RSS 180.52 MiB; footprint 168.24 MiB | Shorter settled sample; RAM misses |
| 10,000 rows, 230 synthetic scroll UI CPU frames | p50 1.025, p95 1.035, p99 1.040, max 1.041 ms | CPU work only; excludes renderer, compositor, display and physical input latency |
| 10 MiB ordinary lines, native source | Usable marker 237.6 ms; approximately 220 MiB RSS after open / 471 MiB after edit/save/undo | This opening marker meets 250 ms; memory misses |
| 10 MiB single line, native source | Usable marker 7,185.7 ms after initial-caret correction; an earlier edit run reached approximately 848 MiB RSS | Responsiveness gate fails; exact same-size edit/save succeeded |

The usable marker requires a completed earlier GUI pass and the initial native text draw, or the complete empty-document shell when no source exists. CLI acknowledgement also includes launcher/socket return. Neither proves that a frame reached the display. The idle interval includes post-readiness settling; no build, profiler or CUA activity ran during it. CPU time uses process task counters with the machine's Mach timebase. Physical 120 Hz delivery is **unverified**.

Profiling a tiny plain-file session found 52 MiB of IOSurface mappings, 14.4 MiB CoreAnimation mappings and approximately 31 MiB resident malloc regions. VM categories and resident/footprint totals are different accounting views and must not be added indiscriminately. These results identify substantial graphics/backing-store costs even before document text. Long-line sampling also reached AppKit text-storage invalidation, glyph generation and CoreText typesetting on the main thread. Tool action/AX batches took seconds, but those durations include automation overhead and are not physical latency measurements.

Implemented cost controls include virtual rows, bounded directory/result/image caches, separate directory/document workers, coalesced recovery, lazy grammar loading, no full-text clone for unknown/plain syntax, cached line indexes, initial caret at the beginning and disabled AppKit background layout. They do not establish a 50 MB result or solve the long-line layout cost. Shrinking the default window, lowering display quality or changing the target was not used to conceal the miss.

## Editor assessment

| Tested component | Concrete evidence | Decision |
| --- | --- | --- |
| egui 0.37 TextEdit | Native shell integration; baseline whole-document layout; the earlier 10 MiB long-line experiment approached 2 GiB RSS | Retained as an explicit development fallback, not qualified for the editing envelope |
| AppKit NSTextView in the Rust shell | Native source accessibility, clipboard, undo, international text and tested grapheme deletion; regular-file editing/save works; long-line results above | Current macOS adapter; ordinary workflow implemented, large-line performance gate failed |
| Floem 0.2.0 text_editor, Vger renderer, isolated release spike | Regular 10 MiB run approximately 155 MiB RSS; 10 MiB single-line first-layout marker 3,094 ms, approximately 6.83 GiB RSS after one inserted character; source absent from accessibility tree | Does not qualify as a replacement: source accessibility and long-line performance both fail this spike |

Floem's first-layout marker is not equivalent to Glimpse's usable-frame marker. The spike was deliberately small and did not implement Glimpse's safety/history/search shell; lower regular-file memory alone cannot justify a migration. These results concern the tested versions and configurations, not every possible implementation of the frameworks. No speculative framework rewrite was undertaken.

## Unverified and failed release gates

Real Japanese/Chinese IME preedit and composition-and-commit remain **unverified by the user's explicit choice**. Automation exposes paste/key presses but no controlled preedit/commit input. Unicode paste, dead keys and grapheme tests do not imply IME success. Full VoiceOver reading/navigation and physical input-to-present likewise remain unverified. Accessibility-tree inspection demonstrates exposed controls and text, not a complete screen-reader qualification.

The memory aspiration and the long-line responsiveness envelope remain failed engineering gates. Warm tiny-file startup success does not qualify cold startup or all supported documents. These are limits of this build, rather than changes to the requested product scope or performance budgets.

## Reference refinement and current aspirations

The current targets are app opening below 100 ms, visually instant folder/file opening, total RAM below 100 MB, and CPU below 0.1% during active use. Active CPU and idle CPU are separate acceptance conditions.

On Mac15,10 / Apple M3 Max / 36 GiB / macOS 27.0, five warm nonactivating release launches measured private readiness acknowledgment at 180–227 ms for a tiny Rust file (p95 227 ms), and 146–158 ms for a folder (p95 158 ms). RSS at acknowledgment was 151–157 MiB for the file and 119–120 MiB for the folder. Nonactivating readiness does not establish physical visible-frame or input latency; cold launch remains unmeasured.

An 85-second intermittent native workflow (Undo/Redo, appearance settings, tabs, tree/columns and Markdown) consumed 0.69% of one core; it includes automation gaps and is not a continuous typing benchmark. A separate 60-second idle interval after that workflow consumed 0.117%, with RSS about 192–200 MiB. All three new resource aspirations remain unmet.

The release CPU benchmark produced p95 1.50 ms / p99 1.72 ms for 230 synthetic 10,000-row scroll frames, and p95 40.55 ms for 20 worker queries over 100,000 cached paths. These exclude renderer/presentation and do not qualify visually instant native navigation. Native checks verified scoped opt-in snapshot search, result activation, exact Find/Replace save, tree/columns return, theme application, clean external refresh and visible-folder changes. Reference screenshots were compared directly; full visual parity remains unqualified. At that checkpoint, settings composition, file-type cues, current-line styling and physical latency still differed or lacked matching-state evidence. Prior long-line, VoiceOver, IME and platform limits remain in force.

## Completed reference follow-up

The native pass exercised the category sidebar, theme application, editor switches, Cmd-L full-path replacement, search arrow/Return activation, nested selection, explicit filename indexing and stale notification, saved-content excerpts, live folder/file refresh and Markdown headings. It exposed and repaired category-layout inheritance, a Read-mode transition into code tabs and ruler-aware narrow source wrapping. The 520-point toolbar, navigator switch, category menu and complete wrapped source were checked in native screenshots. Final wrap checks used nonactivating launches without screen input. The [feature matrix](feature-matrix.md) records actual functionality and remaining qualification.

| Measurement | Result | Limit |
| --- | --- | --- |
| Three final warm tiny-file launches, 1280 × 820 points | Readiness acknowledgment 219–224 ms; RSS 159–160 MiB | Misses 100 ms / 100 MB; excludes physical presentation |
| Three final warm folder launches, same window | 142–169 ms; RSS 118–119 MiB | Misses 100 ms / 100 MB |
| Small native source opens from search/navigation | 24.6–30.6 ms, three observed opens | Request-to-native-draw/UI-pass marker; small local fixtures, no physical input-to-present claim |
| Same visible preview settled into editing | 1.26 ms, one observation | Existing visible buffer; not a newly exposed cached-tab draw measurement |
| In-app folder navigation | 12.0–15.5 ms, four observations | Navigation request to listing available for row layout; excludes renderer/presentation |
| Intermittent active native workflow, 192.89 s | 1.384% of one core; RSS 150→204 MiB | Settings/themes, search/index, navigation/tabs/live refresh/Markdown; includes automation gaps, not continuous typing; active CPU target fails |
| Separate post-interaction quiet interval, 112.48 s, 600 × 650 points, two small tabs | 0.00331% of one core; RSS 190.4 MiB | No new shell frames or native text draws; this smaller-window interval is not a matched comparison to the earlier wide-window idle run |
| Matched clean-source idle, 60 s per binary, 1280 × 820 points | Before 0.0249%, after 0.0220%; RSS 156→164 MiB between builds | Earlier 0.117% post-workflow result was not reproduced; no causal reduction claim |
| 10 MiB single line, final native source | 2,006 ms; RSS 705 MiB at readiness | Earlier matched baseline 7,284 ms / 658 MiB; layout faster, memory worse, both gates still fail |

CPU uses process task counters with the measured Mach timebase, expressed as a percentage of one core. Active and idle intervals remain separate. Warm-cache launch samples do not establish cold startup. These measurements precede the subsequent visual correction; resource usage has not been remeasured after that correction.

Idle diagnosis found no persistent repaint loop in the controlled clean or post-interaction cases. Access-only/other filesystem events are ignored, queued notifications coalesce, and navigation/live-update animation repainting stops after 140 ms (or immediately under Reduce motion). The earlier 0.0483%→0.117% difference remains unexplained across its different session/workflow conditions; the data do not establish that the binary change caused it or that it has been universally eliminated.

A matched after-build memory profile includes 64.1 MiB IOSurface, 16.7 MiB CoreAnimation and about 8 MiB CGImage regions before substantial text. Application setup itself was around 1.6–3 ms while native readiness remained around 200 ms. These profiles leave graphics/backing-store setup and AppKit text layout as material costs. Removing redundant theme installation, avoiding shared-index lock contention and bounding notification/animation work do not overcome those costs. Long-line mode retains the complete editable string and native undo behavior; it is not a virtual text renderer. No speculative framework replacement was adopted to claim the resource targets.

The concrete reference follow-up is implemented and checked, but exact reference parity, physical display latency/120 Hz, cold startup, full VoiceOver, real IME composition and other desktop platforms remain unqualified. The user's IME decision remains unchanged. Startup, total RAM, active CPU and the long-line envelope remain failed aspirations.

## Visual correction verification

The later native pass compared all eight supplied reference surfaces with the actual app at comparable logical sizes. It exercised Cmd-L full-path replacement, scoped text search and Down/Return activation at line 22, Markdown Read/source return, dirty-buffer versus external-disk comparison and cancellation, columns traversal, and 520-point source wrapping/navigation/settings. Test paths, settings and conflict edits were isolated. The local conflict marker was never saved.

The correction changes pane proportions, branded path/search chrome, tab and source insets, explicit heading weight, folder/type symbols, syntax contrast, settings grouping, result hierarchy and conflict composition. The final small changes to wide-toolbar spacing, heading font fallback and three-column fitting were checked in code and packaging after the foreground pass; a subsequent nonactivating capture failed with `cgWindowNotFound`. Earlier native captures show the revised surfaces but do not certify those last adjustments. The columns capture shows two columns and an actual conflict notice, rather than a clean three-column state.

The suite passes 78 tests with two opt-in benchmarks ignored; strict all-target Clippy and signed packaging pass. An initial full run failed the existing filesystem-index freshness test's initial-fresh assertion. That test passed alone and subsequent full runs passed. Production watcher logic was unchanged; an initial filesystem-event race is suspected, not established or repaired by this visual change.

At that checkpoint, remaining visual differences included opaque material, custom vector symbols, modal settings, excerpt comparison without a diff renderer, and CommonMark code-block/complex-page styling. Earlier startup, RAM, active CPU and long-line failures remain applicable; idle measurements do not satisfy the active-use target.

## Typography and line-diff candidate

A subsequent candidate changes new-install source defaults to 14.5 points with a 1.4 minimum line-height factor, comfortable list rows to 32 points, main UI text to 15 points, and settings body/category text to 16/17 points. Existing saved font sizes remain intact; an omitted line-height setting adopts the new default. Source paragraph styling updates attributes without replacing document characters. The default wide window is 1280 × 844 points, matching the reference app's aspect ratio when normalized to that width. Narrow toolbar height is 46 points.

Calibration-candidate nonactivating native captures show 35 reference-equivalent source lines in both wide themes. AppKit reports 14.5-point text and 20.30-point line fragments, versus 17 points in the prior capture and approximately 20 points in references 1/6 after width normalization. Narrow reference 7 is denser (about 19.2 points); the candidate retains the configured source spacing across widths and shows 33 complete line-number bands at 520 × 800 points. These images establish current appearance, not physical visible-frame/input latency.

Yellow search-match color pairs pass automated text contrast checks. Bounded conflict line alignment has regression coverage for replacements, insertions, repeated Unicode lines, missing final newline, empty content, byte/line/character caps and clipped-prefix ambiguity. The suite passes 83 tests with two opt-in benchmarks ignored; strict Clippy and signed packaging pass. The user deferred the consolidated foreground check. Final settings/search/conflict/three-column interaction and exact-binary captures are pending that window; the candidate has not replaced the installed app. No unrelated performance campaign was run.

A later passive capture exposed an unexpected local edit in the isolated test buffer; the owned app was stopped immediately, with its disk fixture unchanged. Winit's process-level activation defaults to ignoring other applications even when the window is configured inactive. The candidate now disables that behavior in background mode and includes a regression for the two activation guards. Native behavior of that repair is pending the approved testing window. No further test launch is attempted while the user works. Line numbers also now follow the source glyph baseline rather than the top of the expanded line fragment. These last changes supersede the calibration captures; final exact-binary captures and native acceptance remain required before installation.
