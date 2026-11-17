# accesskit_consumer

Vendored from AccessKit 0.39.2, upstream commit `f3026222559c7f5f46d8f25cb7404de68eb149f0`. Source: [AccessKit](https://github.com/AccessKit/accesskit/tree/f3026222559c7f5f46d8f25cb7404de68eb149f0/accesskit_consumer).

The local patch applies property-only updates without rebuilding unchanged child relationships. Topology and graft changes use the original validation path. Regression tests compare repeated property updates with the complete topology path; upstream tests remain intact.
