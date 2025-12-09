---
executor: patch
router: patch
---

Avoid panicking when a type reference in another definition cannot be found in the schema during introspection. Instead, log a trace message and return `Null`.