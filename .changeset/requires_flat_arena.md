---
hive-router: patch
---

# Entity `requires` selections are stored in a compact form

The entity key selection on a fetch was stored as a tree of selection items, each 160 bytes, each
owning its own vectors and strings. It is now stored as a flat array of 24-byte nodes addressed by
index, with one table of names shared across the selection, since it is built once and then only
read.

Measured on the two entity fetches this repository's fixtures reach, the selection goes from 665 to
185 bytes of heap - 72.2% smaller - at an unchanged number of allocations. Two fetches is the shape
of the change, not a corpus average.

A stored fetch grows from 208 to 224 bytes, because the compact form is two boxed slices and a
length where the tree was one vector. That is 16 bytes per entity fetch against several hundred
saved on the heap for the same selection. The stored plan node is unaffected.

Query plans on the wire, including the `requires` selections in them, are unchanged byte for byte.
