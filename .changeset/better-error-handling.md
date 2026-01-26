---
router: patch
---

# Better pipeline error handling

- Now there is only one place that logs the pipeline errors instead of many
- Pipeline errors are now mapped automatically from the internal error types using `#[from]` from `thiserror` crate instead of `map_err` everywhere