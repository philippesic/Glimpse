# AccessKit

Vendored from AccessKit 0.25.2, upstream commit `f3026222559c7f5f46d8f25cb7404de68eb149f0`. Source: [AccessKit](https://github.com/AccessKit/accesskit/tree/f3026222559c7f5f46d8f25cb7404de68eb149f0/accesskit).

The local patch shares node properties between clones and copies them when a setter changes a shared node. This avoids duplicating text buffers and property tables during accessibility updates. Serialization retains the property representation through Serde’s `rc` support.
