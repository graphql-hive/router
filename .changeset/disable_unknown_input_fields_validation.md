---
hive-router: minor
graphql-tools: minor
---

Add `validation` configuration options to control input validation.

Input object variables containing unknown fields are rejected by default.

The new `validation.disable_unknown_input_fields_validation` option allows unknown fields in variables and forwards them to subgraphs unchanged. All other validation rules remain enforced, including required fields, type checks, and `@oneOf` constraints. Unknown fields are excluded from `@oneOf` field counting, and inline input values are still validated.

```yaml
validation:
  disable_unknown_input_fields_validation: true
