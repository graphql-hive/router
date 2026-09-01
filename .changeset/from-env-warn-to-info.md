---
hive-router: patch
---

Change `from_env` config fallback log lines from `warn` to `info` level

These logs indicate that a config value fell back to its environment variable, its default, or errored on a missing value — none of which are actually warnings.

Using `info` keeps this visible at the default log level without alarming users.

Fixes https://github.com/graphql-hive/router/issues/1470
