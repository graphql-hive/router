---
hive-router: patch
---

# Response paths in query plans use a simpler stored form

Response paths are built once with the query plan and only read after that, so they now use boxed
slices and boxed strings instead of `Vec`, `String`, and `BTreeSet<String>`.

This makes the stored path types smaller and gives them a simpler owned representation. Type
conditions are still sorted and deduplicated so equivalent paths compare, hash, and serialize the
same way.
