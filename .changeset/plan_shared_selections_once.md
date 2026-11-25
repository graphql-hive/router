---
hive-router: patch
node-addon: patch
---

# Selections shared by several types are planned once

When a query asks for fields on an interface or union, the fields that several types share are planned once for all of them, instead of once per type. Planning large queries is up to three times faster.

A fetch made only for what a `@requires` field needs is dropped when another fetch already gets the same data, so some plans make fewer requests. Entity calls whose fields only differ in order now share one alias, and a provided field that the supergraph doesn't mark `external` no longer makes the planner panic.
