---
hive-router: patch
---

The actual cost of demand control now counts `@cost` on input object fields when the input object comes from a variable, the same as it already did for inline values. It used to count them as 0, so the actual cost came out lower than the estimated cost.
