---
hive-router: patch
node-addon: patch
---

# Variable Coercion Errors Follow the GraphQL Spec

Errors for invalid variable values now use the graphql-js wording, name the variable, and point to the invalid list item. For example, `$input: [String!]` given `[0, 1]` now returns:

```
Variable "$input" has invalid value at [0]: String cannot represent a non string value: 0
```

Values in these messages are printed the way graphql-js prints them (for example `[1]`), not as internal router values. Errors caused by a variable's default value say `has invalid default value`.
