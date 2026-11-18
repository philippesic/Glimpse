# Platform qualification

The GPUI application is **not release-qualified**. Linux is the current qualification target. macOS compilation and native behavior require separate verification; earlier egui/AppKit prototype results do not qualify this implementation. [Glimpse](../glimpse.md) defines the release gates and unchanged performance targets.

## Functional evidence and limits

Native Linux checks use disposable files and isolated settings. Exact bytes, metadata, recovery records and public CLI outcomes are checked independently of the interface. Passing a bounded scenario does not establish every interleaving or the complete workflow tour.

| Area | Exercised behavior | Remaining qualification |
| --- | --- | --- |
| Text preservation | UTF-8, BOM, CR/LF/CRLF and mixed endings, missing final newline, unusual and non-Unicode paths; exact Save and reopen | Complete destination and lifecycle matrix |
| Editing | Grapheme movement/deletion, logical Undo/Redo, Unicode requested positions, selected replacement and supported-size end edits | Complete mouse selection, middle-edit responsiveness and wrap changes |
| International input | Real libpinyin preedit, commit, cancellation, selection replacement and external-refresh deferral; US-intl dead key | Broader composition through focus and shortcut transitions |
| Find | Case and whole-word Unicode matching, replacement, Replace All, Undo/Redo and dismissal focus; dense supported-size correctness | Resource bounds, frame work and adversarial cancellation |
| Markdown | Current unsaved source, selected dialect structures, bounded local images, safe relative links, literal HTML, no automatic remote image fetch, source-position return and stable complete Unicode reading text | Full dialect, long tables and fences, and supported-size resources |
| Coexistence | Same-timestamp replacement, binding validation, clean refresh, dirty conflict and repeated watched saves | Complete delayed and concurrent lifecycle cases on each candidate |
| Persistence | Staged failures, real permission loss, private-mount ENOSPC/read-only/detach, exact retry and Linux metadata | Broader destination races and disconnected-device behavior |
| Recovery | Bound and untitled restart recovery, changed disk contents, live-session isolation and intentional discard | Complete concurrent-session and interrupted-decision matrix |
| CLI and installation | Selected wait/Finish/cancel/cleanup outcomes; installed desktop/CLI launch, icon and MIME registration | Complete picker, independent caller and destination contracts |
| Assistive access | Orca navigation and source editing using the legacy keyboard route. Complete native Wayland Markdown speech through the legacy keyboard route and Orca's public Say All service | Default Wayland key transport and complete independent keyboard and screen-reader tours |
| Layout | Selected source, Find, Preferences, reading and dirty-close tasks across themes, densities, text scales and actual window sizes | Rendered contrast, all menus/decisions, search variants and divider interaction |

## Performance

Large supported documents still exhibit excessive memory and long interface-thread work, particularly with assistive access, dense Find and Markdown. Repeated-use memory growth needs an attributed bound. These are unresolved failures, not consequences excused by correct saved bytes.

Measure each candidate again on Linux. Record the exact binary, features, fixtures, display configuration, cache state, sample distribution, total and settled memory, active/idle CPU, and measurement boundaries. Process readiness is not first editable presentation; worker completion is not a rendered frame; aggregate work per key is not input latency. Private executable-page eviction is not a cold-host launch. Missing GPU accounting is unavailable evidence, not zero usage. Physical input-to-present latency is reported when measurable and is not a release gate.

Use the maintained checks in [README](../README.md), the [native evaluation guide](native-evaluation.md), and the complete gates in the product contract. A required functional or safety check without verification remains failed. Linux evidence does not qualify another platform, and an isolated helper benchmark does not qualify the active GPUI workflow.
