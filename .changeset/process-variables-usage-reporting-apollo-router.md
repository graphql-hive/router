---
apollo-router-hive-fork: minor
---

Add `process_variables` to Hive Console usage reporting

By default, usage reports mark every field of an input-object variable's declared type as used, because the SDK cannot know which fields a client actually sends.

With `plugins.hive.usage.process_variables: true`, the Apollo-Router plugin takes the request's variables at the supergraph stage, before execution, and converts them once per report.

In either mode, the variable values are never sent to Hive Console, only schema coordinates.

Defaults to `false`;

Closes https://github.com/graphql-hive/router/issues/1488
