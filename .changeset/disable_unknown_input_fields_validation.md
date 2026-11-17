---
hive-router: minor
graphql-tools: minor
---

Add `validation` config with an opt-out for unknown input fields

Since input object variables are validated, a variable holding a field its type doesn't define is rejected. Clients often send back objects they got from the API, with `__typename` still in them, and those requests started to fail.

`validation.disable_unknown_input_fields_validation` lets them through again. The variable goes to the subgraphs as sent, unknown fields included. Everything else is still checked: missing required fields, wrong types and `@oneOf` rules, where unknown fields don't count towards the one field. Values written inline in the operation are still validated.

```yaml
validation:
  disable_unknown_input_fields_validation: true
```

The `ROUTER_DISABLE_INT_VALUE_RANGE_VALIDATION` environment variable now has a config counterpart too. The environment variable still works and wins over the config file.

```yaml
validation:
  disable_int_value_range_validation: true
```

`ValidationOptions` in `graphql-tools` has a new `disable_unknown_input_fields_validation` field. It's only read when coercing variables, not by `ValuesOfCorrectType`.
