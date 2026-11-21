# Glimpse patches

This is the Apache-2.0 GPUI 0.2.2 crate from crates.io, with its platform sources and resources retained.

The Linux Wayland keyboard handler retains modifier masks received before a keymap and applies them when the keymap arrives. Key events without a keymap are ignored safely. This prevents a startup panic observed while restarting under a headless Sway session with a persistent virtual keyboard. Other platform implementations are unchanged by this patch.

Shared text layout includes truncation width in its measurement cache and retains original style runs between measurement passes. This lets constrained labels render an ellipsis after an initial unconstrained layout instead of reusing the untruncated measurement.

Highlight styles carry a `monospace` flag. A run with the flag uses the family registered with `set_monospace_family`, so inline code in rich text can use the source face without making `HighlightStyle` non-`Copy`.
