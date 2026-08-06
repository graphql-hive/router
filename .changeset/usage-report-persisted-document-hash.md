---
hive-router: patch
---

# Report `persistedDocumentHash` in usage reports

Usage reports now include the resolved persisted document id, so Hive Console can match
requests to app deployments and populate their "Last used" data. Previously the router
resolved the document id but always omitted it from the usage report.

Closes https://github.com/graphql-hive/router/issues/1343
