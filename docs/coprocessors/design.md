# Coprocessors in Hive Router

To make it easy for people to migrate, we will offer a similar solution to Coprocessors in Apollo Router.

## Stages

### Router
Identical to Apollo’s Router stage. This stage represents the inbound http request and the response sent to the client.

```yaml
request:
  body: false
  context: false
  headers: false
  method: false
  path: false
response:
  body: false
  context: false
  headers: false
  status_code: false
```

### GraphQL

Identical to Apollo’s Supergraph stage. This stage represents the inbound graphql request and the response sent to the client. It’s basically the same as the Router stage, but triggered only for http requests hitting the graphql endpoint, and additionally the schema is already known at this point.

**Ordering requirement:** GraphQL coprocessor stages must run before query planning in this order:

1. `stages.graphql.request`
2. normalize/analysis in router
3. `stages.graphql.analysis`

`stages.graphql.analysis` runs before progressive override, authorization, future cost-demand checks, and query planning.

`stages.graphql.analysis` must treat GraphQL request body as read-only. It may only mutate headers/context.
`include.body` accepts `true`, `false`, or a list of fields (`query`, `operation_name`, `variables`, `extensions`). `body: []` behaves the same as `false`.
`include` controls only what the router sends to coprocessor. Valid mutations returned by coprocessor can still be applied even if that field was not included in the outbound payload.

```yaml
request: 
  body: false # true | false | [query, operation_name, variables, extensions]
  context: false
  headers: false
  method: false
  path: false
  sdl: false
analysis:
  body: false # true | false | [query, operation_name, variables, extensions]
  context: false
  headers: false
  method: false
  path: false
  sdl: false
response:
  body: false
  context: false
  headers: false
  status_code: false
  sdl: false
```

### Execution

Identical to Apollo's Execution stage. This stage represents the execution of the plan and the response prepared by requests to the subgraphs. At this point the GraphQL request was parsed, validated and a query plan was prepared.

```yaml
request: 
  body: false # inboud request
  context: false
  headers: false # inboud request
  method: false # inboud request
  path: false # inboud request
  sdl: false
response:
  body: false
  context: false
  headers: false
  status_code: false
  sdl: false
```

### Subgraph

Identical to Apollo’s Subgraph stage. This stage represents the outbound graphql http requests and the responses sent by the subgraphs.

```yaml
request:
  body: false # outbound request's body
  context: false
  headers: false  # outbound request
  method: false # outbound request
  uri: false # outbound request
  sdl: false
  service_name: false
  subgraph_request_id: false
response:
  body: false
  context: false
  headers: false
  status_code: false
  sdl: false
  service_name: false
  subgraph_request_id: false
```

### Dynamic opt-in/out

To reduce unnecessary traffic to a coprocessor, users may want to trigger requests only of those sent by unauthenticated users, or under certain conditions.
Apollo Router offers condition property to perform logical operations on data, to decide whether or not an event needs to be prepared and sent to a coprocessor.
We should offer a similar thing, but with VRL expressions instead.

### Stages in Coprocessor vs Hooks in Plugins

Native plugin system has much more granular control over the pipeline for many reasons.

* `Plugin::on_http_request` = `stages.router` - any http request
* `Plugin::on_graphql_params` = `stages.graphql.request` - graphql request before normalization
* `Plugin::on_graphql_parse` = No equivalent stage, as we can’t "parse over the wire"
* `Plugin::on_graphql_validation` = No equivalent stage, as validation can be done in `stages.graphql.request` (we send everything including public schema sdl)
* `Plugin::on_query_plan` = Closest equivalent is `stages.graphql.analysis` (runs before query planning, auth/progressive override/cost-demand decisions)
* `Plugin::on_execute` = `stages.execution` - graphql execution is about to happen
* `Plugin::on_subgraph_execute` = No equivalent stage, as what would we even allow the coprocessor to do here.
* `Plugin::on_subgraph_http_request` = `stages.subgraph` - http request to a subgraph
* `Plugin::on_plugin_init` - No equivalent stage, as there’s no reason to have one
* `Plugin::on_supergraph_load` - No equivalent stage, as we ship sdl in coprocessor request body on demand, and keeping the supergraph in the coprocessor can lead to race conditions.
* `Plugin::on_shutdown` - No equivalent stage, as there’s nothing to intercept here, and the coprocessor service will receive a shutdown signal from the k8s (or similar) anyway.
* `Plugin::on_graphql_error` - No equivalent stage, but users can intercept the errors in the stages.graphql response that is sent to the client.

## Network and Communication

It has to be super efficient as it lives in a hot path and it has to be capable of short-circuiting the request pipeline.

### Network

Requirements:
* Keep per-call overhead as low as possible
* Support high concurrency with many in-flight router requests
* Use a transport that is widely supported by common languages and HTTP servers
* Support http/1 and http/2 with and without tls (h2c for http/2 without tls)
* Support Unix Domain Sockets


**Why http/1?**
Why not...

**Why h2c?**
For same-host and same-pod deployments, h2c avoids TLS overhead while still enabling HTTP/2 multiplexing.
Some frameworks/http server implementations may lack h2c support, so it should be supported in Hive Router, but as opt-in and very explicit.

**Why UDS?**
Unix Domain Socket avoids unnecessary network stack overhead for same-host communication while remaining compatible with standard HTTP-style servers.
Without UDS, the OS goes through IP, TCP and routing layers.
With UDS, the OS skips IP layer and stays inside kernel and do not pretend this is a network call.
It’s important to check whether the socket file should be deleted or not, and by who.
I think it should be coprocessor’s responsibility as the router is basically a client.

**Why TLS?**
Sidecar does not need TLS, but in some cases users may deploy it outside pod or host it in weird places...

### Flow control

Coprocessor should be able to short-circuit the client request. We should offer break and continue.
The question is wether we should require status code or not and I (@kamil) think we should - if you have the power to break the flow and send back a response, the response should include the status code and body to inform the user what happened.
Apollo Router expects: `{ "control": { "break":  400 } }` .
I like the idea of having a property "control", but the `{"break": <status code>}` as the value is a bit weird.
It has only the status code, but no body.
I understand why they did it, as there’s "body" property in the response that coprocessor can use to manipulate the body.
The status code on the other hand is only available as “statusCode” in *Response stages, but Apollo Router does not allow to modify it.
I think it’s reasonable to modify the status code only in case of early returns (short-circuit scenario).

Imo we can adopt this pattern and make the Apollo Router → Hive Router transition smoother, and in case we see issues (that I cannot see at the moment), we can introduce "version": 2.

Apollo Router expects `"control": "continue"` in responses from coprocessor, but I think it’s reasonable to assume it’s a continuation if it has no `"control": { "break": <status code> }`. This way control property is optional.

### Communication protocol

Every protocol needs a version to define the “we’re talking the same language” contract.
We will require from coprocessor to send back "version": 1.
This is needed in order for us to introduce break changes both in syntax/data structure and behaviour.

Hive Router expects a response body in the structure matching the request body sent to the coprocessor.
When coprocessor requests a property (e.g. “body”), but does not modify it, then Hive Router should not expect that property to be sent back. When the response from the coprocessor lacks the property, Hive Router should use the existing value.
If the property’s value is set to null or {} or any other value, Hive Router should treat that as a new value and modify the state.
That applies to top-level fields only, not nested structures - when "context" is sent to Hive Router by a coprocessor, it should be treated as “this is entirely new value” and require validation.
Hive Router should ignore the rest of fields sent by a coprocessor, to save on deserialization cost.


## Observability

It’s core piece of the system so we need to provide visibility to users.

### Metrics

We should track duration and error rate per stage, to observe performance and healthiness.
Additionally, we can track evaluation of the condition property, per stage, to observe effectiveness, but I think we should add it only after we receive such feature request from users.

### Traces

We need dedicated spans with context propagation for http requests sent to coprocessors.
Span has to include information about the stage.

### Logging

We should info log what was modified by the coprocessor, what properties.
We should info log when coprocessor short-circuit the request and with what status code.
We should error log when coprocessor failed to respond (or timed out).
We should debug log requests to coprocessors.

**Log correlation**
Since the coprocessor is a separate process, we need to make sure user is able to connect logs from Hive Router with logs from the coprocessor service.
Every stage should have a unique identifier assigned, that is sent to the coprocessor.
Apollo Router uses "id" property that represents “A unique ID corresponding to the client request associated with this coprocessor request”, but I have no idea what they mean by “client request’.
Is it the inbound request sent by graphql client or by “client” they mean the Apollo Router...
Hive Router should send the unique id of the coprocessor event as "id" and id related to the inbound request as “request_id” (or under different name), and when there’s an outbound request (to subgraph) “subgraph_request_id”.
At least two values should be sent plus extra subgraph request id for subgraph stage.



## Context

Apollo Router has:
- Authentication
  - `apollo::authentication::jwt_claims` - Claims extracted from a JWT if present in the request
  - `apollo::authentication::jwt_status` - JWT validation status (internal only)
- Authorization
  - `apollo::authorization::authentication_required` - true if the operation contains fields marked with @authenticated
  - apollo::authorization::required_scopes` - If the operation contains fields marked with @requiresScopes, it contains the list of scopes used
  - `apollo::authorization::required_policies` - If the query contains fields marked with @policy, it contains a map of policy name -> Option<bool>. A coprocessor or Rhai script can edit this map to mark true on authorization policies that succeed or false on ones that fail 
- Progressive override
  - `apollo::progressive_override::unresolved_labels` - List of unresolved labels
  - `apollo::progressive_override::labels_to_override` - List of labels for which overrides are needed

### Progressive Override (Hive keys)

Hive Router context should support progressive override using flat namespaced keys:

- `hive::progressive_override::unresolved_labels`
- `hive::progressive_override::labels_to_override`

This data is exchanged as top-level context keys (not a nested `progressive_override` object).
Both values must be arrays of strings.

### Implementation notes (deferred)

- Coprocessor context synchronization uses `std::sync::Mutex` as the single lock strategy.
- Add strict serialization/deserialization for context progressive override keys in coprocessor context handling.
- Keep the shape flat and namespaced; reject legacy nested `progressive_override` payloads.
- Ensure `graphql.request` can send current context and apply returned context mutations.
- Ensure mutated context from `graphql.request` is available before query planning starts.
- Add tests for:
  - round-trip of flat namespaced keys,
  - invalid value types,
  - legacy nested shape rejection,
  - duplicate/reserved key handling.
