---
router: major
---

# Refactor Router Initialization Error Handling in `hive-router`

- New `RouterInitError` enum to represent initialization errors in the Hive Router.
- `router_entrypoint` now returns `Result<(), RouterInitError>` instead of a boxed dynamic error(`Box<dyn std::error::Error>`), providing more specific error handling during router initialization.
