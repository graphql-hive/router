---
hive-router: patch
node-addon: patch
---

# Make query plan cache keys more robust

The query plan cache now uses a 128-bit BLAKE3 digest over an injective encoding of the operation. This guarantees that distinct operations always map to distinct cache keys.
