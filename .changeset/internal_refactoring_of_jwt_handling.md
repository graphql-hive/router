---
router: patch
---

# Internal refactoring of JWT handling

Passing mutable request reference around was the unnecessary use of `req.extensions` to pass `JwtContext`. 

Instead, we can directly pass `JwtContext` as-is instead of using `req.extensions`.
