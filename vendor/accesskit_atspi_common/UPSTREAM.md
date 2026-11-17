# accesskit_atspi_common

Vendored from AccessKit 0.21.1, upstream commit `f3026222559c7f5f46d8f25cb7404de68eb149f0`. Source: [AccessKit](https://github.com/AccessKit/accesskit/tree/f3026222559c7f5f46d8f25cb7404de68eb149f0/adapters/atspi-common).

The local patch caches scalar offsets for flat leaf text runs, limits text-change comparisons to edited runs and avoids re-enumerating their unexposed children. Nested, masked, character-metadata and structural changes retain the complete traversal path. Regression tests check Unicode text events, caret positions, structural invalidation and exposed focus/controls.
