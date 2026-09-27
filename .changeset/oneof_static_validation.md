---
graphql-tools: minor
hive-router: patch
---

# Validate `@oneOf` literals and variables

Validation now enforces the GraphQL spec's rules for `@oneOf` input objects in operations:

- `ValuesOfCorrectType` rejects a `@oneOf` literal that doesn't set exactly one field (`OneOf Input Object "OneOfInput" must specify exactly one key.`), or sets it to `null` (`Field "OneOfInput.a" must be non-null.`). This covers arguments and variable default values.
- `VariablesInAllowedPosition` rejects a nullable variable used as the field of a `@oneOf` literal, such as `query ($a: String) { field(input: { a: $a }) }`, even when the variable has a default value (`Variable "$a" is of type "String" but must be non-nullable to be used for OneOf Input Object "OneOfInput".`).
