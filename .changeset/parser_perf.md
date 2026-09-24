---
graphql-tools: patch
---

# Refactor GraphQL parser performance paths

Keep the hand-written recursive-descent query parser and byte-oriented tokenizer, while making
lookahead, checkpoint replay, token accounting, nesting limits, positions, and string escapes
explicit. The old query `combine` grammar is removed and its fixture coverage now exercises the
public parser.

This also fixes two deliberate token-limit behaviors: strings count as tokens, and input ending
exactly at the configured token limit is accepted. No public AST or function signatures change.
