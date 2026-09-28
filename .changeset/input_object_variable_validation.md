---
hive-router: major
---

# Validate input object variables

Variable values of input object types are now validated before the operation runs, as the GraphQL spec requires. Before, any value was accepted and forwarded to the subgraphs as-is.

The Router now rejects an input object variable when:

- the value is not an object: `Expected value of type "ReviewInput" to be an object, found: "great".`
- a required field is missing (a non-null field without a default value): `Expected value of type "ReviewInput" to include required field "stars", found: { comment: "great" }.`
- it contains a field the type doesn't define: `Expected value of type "ReviewInput" not to include unknown field "rating", found: { stars: 5, rating: 5 }.`
- a field's value is invalid for its type. The error points to the field, including inside nested objects and lists: `Variable "$review" has invalid value at .tags[1]: String cannot represent a non string value: 1`

Field default values are still applied by the subgraph: the Router forwards the object as sent. The ensures the subgraph has the ability to identify a non-provided variable.
