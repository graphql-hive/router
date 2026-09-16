---
hive-console-sdk: minor
hive-router: minor
---

Add `process_variables` to Hive Console usage reporting

By default, usage reports mark every field of an input-object variable's declared type as used, because the SDK cannot know which fields a client actually sends.

With `telemetry.hive.usage_reporting.process_variables: true`, the router reads request's raw variables (taken before coercion) with its usage report, and the usage report lists only the input fields present in the payload.

In either mode, the variable values are never sent to Hive Console, only schema coordinates.

Defaults to `false`;

Closes https://github.com/graphql-hive/router/issues/1488
