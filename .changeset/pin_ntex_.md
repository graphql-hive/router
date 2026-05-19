---
hive-router: patch
hive-router-internal: patch
hive-router-plan-executor: patch
hive-console-sdk: patch
hive-apollo-router-plugin: patch
---

# Fix: pin `ntex` version to `3.7.2` to avoid regressions

This release pins `ntex` to `3.7.2` to avoid regressions, like the one reported in [#997](https://github.com/graphql-hive/router/issues/997). 

Users who builds their own router are impacted by this regression, due to the way Cargo handles unpinned dependencies.
