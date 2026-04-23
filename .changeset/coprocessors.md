---
hive-router: minor
hive-router-internal: minor
hive-router-plan-executor: minor
hive-router-config: minor
---

## Coprocessors

Introduces Coprocessors as language agnostic way to extend Hive Router.

**Supports coprocessor stages:**
- `router.request`
- `router.response`
- `graphql.request`
- `graphql.analysis`
- `graphql.response`

**Stage capabilities:**
- include selected request/response fields in stage payloads (headers, body, context, and optional SDL depending on stage config)
- mutate request body/headers/context for downstream pipeline execution
- short-circuit and return an immediate HTTP response from a stage

**Transport and endpoint support:**
- `http://` and `unix://` (unix socket domain) endpoints
- http/1, http/2 and h2c protocols

**Error handling:**
- coprocessor failures map to server-side failures (500)
- client-facing GraphQL errors are masked as Internal server error
- structured error codes are preserved in GraphQL extensions.code
- detailed coprocessor failure reasons remain in server logs/telemetry only

**Adds coprocessor metrics:**
- hive.router.coprocessor.requests_total
- hive.router.coprocessor.duration
- hive.router.coprocessor.errors_total
