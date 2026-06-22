---
hive-router-plan-executor: patch
hive-router: patch
---

# Correctly process variables in introspection execution

Introspection resolution only handled inline literal arguments, so arguments
passed as variables (e.g. `__type(name: $name)` or `includeDeprecated: $flag`)
were ignored and resolved to their defaults.

The introspection context now carries the request variables.

Fixes [#1185](https://github.com/graphql-hive/router/issues/1185)
