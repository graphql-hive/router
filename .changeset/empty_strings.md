---
router: patch
config: patch
internal: patch
executor: patch
---

# Treat empty strings as None for environment variables

For example when the user sets `HOST=""`, we now treat it as if the user did not set the variable at all.