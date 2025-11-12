---
router: patch
config: patch
executor: patch
---

# Persisted Documents

- Supports Hive's `documentId` spec, Relay's `doc_id` spec and Apollo's `extensions` based spec as options
- - It is also possible to use your own method to extract document ids using VRL expressions
- Hive Console and File sources are supported
- A flag to enable/disable arbitrary operations
- - A VRL Expression can also be used to decide this dynamically using headers or any other request details

[Learn more about Persisted Documents in the documentation.](https://the-guild.dev/graphql/hive/docs/router/configuration/persisted_documents)