---
hive-router: patch
---

# Entity `requires` selections use less memory

A fetch stores the fields it needs to build an entity representation. These selections used to be
stored as a tree, where each item was 160 bytes and had its own vectors and strings.

They are now stored as a flat array of 24-byte nodes. Nodes refer to each other by index and share
one table of names. This works well because the selection is built once and only read after that.

A stored fetch itself grows from 208 to 224 bytes. The new format needs two boxed slices and a
length instead of one vector.
