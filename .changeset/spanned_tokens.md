---
graphql-tools: minor
---

# Add `spanned_tokens` to split a document into tokens with their byte ranges

`graphql_tools::parser::spanned_tokens` returns every token of a GraphQL document with its kind (`TokenKind`) and the byte range it covers. A string or block string is a single token.
