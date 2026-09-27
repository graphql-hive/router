---
hive-router: major
---

# Validate `@oneOf` input object variables

Variable values of `@oneOf` input object types are now validated before the operation runs. A value must set exactly one field, and that field must not be `null`. Before, any value was accepted and forwarded to the subgraphs.

For example, `{"input": {}}`, `{"input": {"a": "abc", "b": 123}}` and `{"input": {"a": null}}` are now rejected with:

```
Variable "$input" has invalid value: Within OneOf Input Object type "OneOfInput", exactly one field must be specified, and the value for that field must be non-null.
```

This also applies to `@oneOf` objects nested in other input objects or lists, and to variable default values.

Closes https://github.com/graphql-hive/router/issues/1390
