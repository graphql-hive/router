---
hive-router: patch
---

# Entity `requires` selections are stored in a compact form

A fetch stores the entity key selection it needs from a subgraph. It used to be stored as a tree
of selection items. Each item was 160 bytes and owned its own vectors and strings.

It is now stored as a flat array of 24-byte nodes that refer to each other by index, with one
table of names shared across the whole selection. This works because the selection is built once
and then only read.

Measured on the two entity fetches this repository's fixtures reach, the selection goes from 665 to
185 bytes on the heap. That is 72.2% smaller, with the same number of allocations. Two fetches is a
small sample, so read this as the shape of the change rather than an average.

A stored fetch grows from 208 to 224 bytes, because the compact form holds two boxed slices and a
length where the tree held one vector. That is 16 bytes more per entity fetch, against several
hundred bytes saved on the heap for the same selection. The stored plan node does not change.

Query plans on the wire, including the `requires` selections in them, are unchanged byte for byte.
