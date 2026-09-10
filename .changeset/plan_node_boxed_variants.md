---
hive-router: patch
---

# Smaller query plan nodes in the plan cache

A plan node is an enum, so every plan node takes as much space as its largest variant. This is
true even for the slots in a sequence or a parallel list that hold something small.

Four of the variants carry a subgraph fetch, and those four were much larger than the rest. They
set the size for every plan node.

Those four variants are now boxed, so the large data is stored separately and the node itself stays
small. A stored plan node goes from 224 to 40 bytes. A plan holds one node per step, so the saving
grows with the size of the plan rather than with the number of fetches.

The largest variant is now the condition node. A test checks its size, so it stays visible if it
grows.

Query plans on the wire are unchanged. Boxing a variant does not change how it is serialized.
