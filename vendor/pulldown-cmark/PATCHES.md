# Glimpse patch

Pulldown-cmark 0.13.5 stores parse tree nodes in pages. Consuming a top-level block releases pages that precede the next block. The offset iterator reads the closing span before the old page can be released. After emitting a link or image start event, the parser reuses its consumed link slot. Reference definitions and the two parsing passes retain their existing behavior.
