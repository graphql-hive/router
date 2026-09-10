---
hive-console-sdk: major
hive-router: minor
apollo-router-hive-fork: minor
---

Add `process_variables` support to Hive Console usage reporting

When enabled, input-object and enum variables are reported based on the fields actually present in each request's runtime variables payload, instead of conservatively marking every field of the declared type as used. The content of the variables is never sent — only the schema coordinates it touches.

Enable it via `usage_reporting.process_variables: true` (hive-router) or `process_variables: true` on the Hive usage plugin (apollo-router fork). 

Defaults to `false`, preserving the existing conservative behavior.
