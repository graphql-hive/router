---
graphql-tools: minor
---

# `@defer` validation rules

Two rules from graphql-js 17, for servers that support `@defer`. They aren't in the default rules, add them to a validation plan:

- `DeferDirectiveLabel`: a `label` has to be a static string, unique across every `@defer` in the document.
- `DeferDirectiveOnRootField`: no `@defer` on the root selection set of a mutation or subscription.

`@stream` isn't checked.
