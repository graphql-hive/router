---
hive-router: patch
node-addon: patch
---

# Fix progressive `@override` on fields returning a union

When a field returning a union was progressively overridden (`@override(from: ..., label: ...)`), the overriding subgraph was still used when the label was off. The label is now respected, like it is for other fields.
