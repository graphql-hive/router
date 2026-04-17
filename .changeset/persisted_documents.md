---
hive-router-plan-executor: minor
hive-router-config: minor
hive-router: minor
hive-router-internal: minor
hive-console-sdk: minor
hive-apollo-router-plugin: patch
---

# Persisted Documents

Introduces persisted documents support in Hive Router with configurable extraction and storage backends.

Supports extracting persisted document IDs from:
- `documentId` in request body (default)
- `documentId` in URL query params (default)
- Apollo-style `extensions.persistedQuery.sha256Hash` (default)
- custom `json_path` (for example `doc_id` or `extensions.anything.id`)
- custom `url_query_param` (for example `?doc_id=123`)
- custom `url_path_param` (for example `/graphql/:id`)

Order is configurable and evaluated top-to-bottom.

Supports persisted document resolution from:
- file manifests (Apollo and Relay KV styles)
- Hive CDN (via `hive-console-sdk`)

File storage includes watch mode by default (with 150ms debounce) to reload manifests after file changes.
Hive storage validates document ID syntax before generating CDN paths to avoid silent invalid-path behavior.

Adds persisted-documents metrics:

- `hive.router.persisted_documents.extract.missing_id_total`
- `hive.router.persisted_documents.storage.failures_total`

These help track migration progress and resolution failures in production
