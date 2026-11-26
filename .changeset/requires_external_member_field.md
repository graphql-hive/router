---
hive-router: patch
node-addon: patch
---

# Required fields come from a subgraph that resolves them for that type

An `@interfaceObject` field with `@requires` needs the required fields for each type of the interface. When the interface's subgraph has the field but one of its types marks it `@external` there, the plan still asked that subgraph for it, for objects of that type too, and got nothing back. Now those objects get the field from a subgraph that resolves it for their type, like a query asking for the field on the interface does.
