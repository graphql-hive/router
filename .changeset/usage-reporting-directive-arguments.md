---
hive-console-sdk: patch
---

Stop reporting directive arguments as schema coordinates in usage reports

Arguments of directives applied to fields, such as `users @include(if: $flag)`, were collected as field-argument coordinates (`Query.users.if`).

Directive arguments are not schema coordinates, so usage reports now skip them as well.
