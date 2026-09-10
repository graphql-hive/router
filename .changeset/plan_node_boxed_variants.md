---
hive-router: patch
---

# Smaller query plan nodes in the plan cache

PlanNode is an enum, so every variant takes as much space as its largest variant. 
Some of the variants have a subgraph fetch, and those were much larger than the rest. 
They are now boxed.
