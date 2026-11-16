# Vendored dependencies

`gpui-component` is the published 0.5.1 source from crates.io, licensed under Apache-2.0. Glimpse adds an optional external-highlight vector and setter to InputState, and clips/composes those styles with the default foreground during visible-line rendering. This keeps syntect as Glimpse's syntax provider. A cursor-blink setter lets Glimpse use a steady caret without scheduling periodic redraws; other inputs retain the upstream default. The editor stores line lengths without per-line rope copies, prepares wrapping before first layout, and shapes viewport rows plus selection endpoints while retaining full byte offsets and scroll geometry. Full-line wrapping and buffer extraction still require performance qualification.

`xattr` is the published 0.2.3 source, licensed under MIT or Apache-2.0. Its Linux missing-attribute constant uses ENODATA; libc no longer exposes the obsolete ENOATTR alias on Linux aarch64. GPUI's archive dependency still selects this version.

Licenses are retained in each directory. Changes should remain small enough to compare directly against the named crate release.

The component is a workspace member so `cargo test --locked --workspace` runs its unit tests alongside Glimpse’s tests. The docking fixture omitted from the published crate is retained from [the matching upstream tag](https://github.com/longbridge/gpui-component/blob/v0.5.1/crates/ui/tests/fixtures/layout.json). Its formatter configuration preserves upstream import order. Optional component features remain disabled in the application.
