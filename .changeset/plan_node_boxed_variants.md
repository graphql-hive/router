---
hive-router: patch
---

# Smaller query plan nodes in the plan cache

A plan node is an enum, so every one of its slots is as large as its largest variant - including the
slots in a sequence or a parallel list that hold something small. The four variants that carry a
subgraph fetch were an order of magnitude larger than the rest and set that size for all of them.

Those four are now boxed. A stored plan node goes from 224 to 40 bytes, and a plan holds one per
node, so the saving scales with plan size rather than with fetch count. The new ceiling is the
condition node, which is pinned by a test so it stays visible if it grows.

Plans on the wire are unchanged - boxing is invisible to serialization.
