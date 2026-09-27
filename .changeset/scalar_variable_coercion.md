---
hive-router: patch
---

# Stricter and more complete coercion of scalar variables

Scalar variable values are now checked and validation using the following rules:

- `Int` accepts only 32-bit integers. A value like `2147483648` is now rejected with `Int cannot represent non 32-bit signed integer value: 2147483648`, instead of being forwarded to the subgraph.
- `ID` now accepts integers as well as strings, as the GraphQL spec requires. This also fixes operations like `query ($id: ID = 1)`, which failed whenever `$id` was omitted.
- `Float` now accepts every JSON number, including integers larger than `9223372036854775807`.
- `Int` and `ID` still reject JSON numbers written as floats, like `1.0` or `1e3`. Error messages now print these values with their `.0` (`Int cannot represent non-integer value: 1.0`).
