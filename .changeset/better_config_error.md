---
router: patch
config: patch
---

# Better error handling for configuration loading

- In case of an invalid environment variables, do not crash with `panic` but provide a clear error message with a proper error type.
- In case of failing to get the current working directory, provide a clear error message instead of crashing with `panic`.
- In case of failing to parse the configuration file path, provide a clear error message instead of crashing with `panic`.