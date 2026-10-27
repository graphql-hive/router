---
hive-router: minor
---

# `hive_router anonymize` command

Anonymizes a supergraph and its operations with a secret key, so you can share them without revealing the schema.

```bash
hive_router anonymize keygen > anon.key
hive_router anonymize supergraph --key-file anon.key supergraph.graphql > anonymous.graphql
hive_router anonymize operations --key-file anon.key operations/ anonymous-operations/
```

Every type, field, argument, enum value, subgraph name, string and number goes through keyed BLAKE3, so the output is a valid supergraph that plans the same way, and nobody can reverse it without the key. The same key always gives the same names, so operations can be anonymized later on their own, without touching the supergraph again.
