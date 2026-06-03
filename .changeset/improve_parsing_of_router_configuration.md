---
hive-router-config: patch
hive-router-internal: patch
hive-router-plan-executor: patch
hive-router: patch
---

# Improve parsing of Router configuration

Improve error messages when parsing Router configuration, in cases where `SingleOrMultiple<T>` is used and parsing of `T` fails. 

The error is now visible to the user, instead of being swallowed and reported as a generic error.
