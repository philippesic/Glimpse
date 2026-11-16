# Vendored dependencies

`gpui-component` is the published 0.5.1 source from crates.io, licensed under Apache-2.0. Glimpse adds an optional external-highlight vector and setter to InputState, and clips/composes those styles with the default foreground during visible-line rendering. This keeps syntect as Glimpse's syntax provider. Other component behavior remains upstream code.

`xattr` is the published 0.2.3 source, licensed under MIT or Apache-2.0. Its Linux missing-attribute constant uses ENODATA; libc no longer exposes the obsolete ENOATTR alias on Linux aarch64. GPUI's archive dependency still selects this version.

Licenses are retained in each directory. Changes should remain small enough to compare directly against the named crate release.
