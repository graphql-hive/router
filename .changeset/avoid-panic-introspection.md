---
executor: patch
router: patch
---

Avoid panicking when a type reference in another definition cannot be found in the schema during introspection. Instead, log a trace message and return only `name` field for that reference and `null` for the rest.