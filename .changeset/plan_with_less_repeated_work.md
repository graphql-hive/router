---
hive-router: patch
node-addon: patch
---

# Less repeated work when planning

Planning large queries does less of the same work twice. An entity call for every possible type at the same spot is made once, instead of once per type and merged afterwards. A search for how to reach a field is reused when it starts from the same place, and building the fetch graph copies and hashes less.

Some plans list the same fetches in a different order. Nothing else about them changes.
