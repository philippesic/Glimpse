# Glimpse patches

This is the Apache-2.0 GPUI 0.2.2 crate from crates.io, with its platform sources and resources retained.

The Linux Wayland keyboard handler retains modifier masks received before a keymap and applies them when the keymap arrives. Key events without a keymap are ignored safely. This prevents a startup panic observed while restarting under a headless Sway session with a persistent virtual keyboard. Other platform implementations are unchanged by this patch.
