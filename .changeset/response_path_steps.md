---
hive-router: patch
---

# Response paths in a query plan own their steps

A flatten node's path, and the merge paths on a batched entity fetch, were a `Vec` of steps whose
field names were `String`s and whose type conditions were an inline `BTreeSet<String>`. A path is
built once when the plan is built and then only read, so it is now a boxed slice of steps that own
exactly what they hold: a boxed field name, and a boxed type condition of boxed names. Code that
walks a path takes a borrowed view of it rather than a reference to the owner.

A stored path step goes from 32 to 24 bytes, a stored path from 24 to 16, and the merge-path list on
an entity batch alias from 24 to 16. This is not a memory change overall: boxing the type condition
adds an allocation for each step that has one, roughly offsetting the smaller steps. It is a change
to how paths are held, made for the type-level guarantees below.

The type names in a condition are sorted and deduplicated when the condition is built, whichever
direction it is built from - the planner, or deserialization of a plan that came back from a plugin
or `node-addon`. `main` gets that from `BTreeSet` for free; a plain slice does not, and two paths
that differ only in the order their type names were written would then compare, hash and serialize
differently, and a batched entity fetch would walk the same path twice instead of sharing it. Three
tests pin the normalization so the guarantee survives the change of representation.

Query plans on the wire are unchanged, byte for byte.
