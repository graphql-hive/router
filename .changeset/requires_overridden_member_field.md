---
hive-router: patch
node-addon: patch
---

# Required fields follow `@override` on the interface's types

An `@interfaceObject` field with `@requires` needs the required fields for each type of the interface. When one of those types has the field moved to another subgraph with `@override`, the plan still read it from the interface in the old subgraph for objects of that type. Now it follows the override. A progressive `@override(label: ...)` on a type of an interface was also ignored when a query read the field through the interface, `{ accounts { title } }` or `... on Admin { title }`. Now the field comes from whichever subgraph the request goes to.
