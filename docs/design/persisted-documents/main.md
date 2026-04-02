# Persisted Documents in Hive Router

I’m planning to implement this feature in Hive Router.

## HTTP Request (Input)

There is only one piece of data (Hive CDN is an exception here) that Hive Router needs from the graphql client, it’s the identity of the document. I will call it “document id”, but in reality it could be anything: a hash, custom string, combination of both.
This document id can be included in the HTTP Request in many ways:

* URL
    * `/graphql/<id>`
    * `/graphql?id=<id>`
* Header
    * `graphql-document-id: <id>`
* Body
    * `{ "document_id": <id> }`
    * `{ "extensions": { "whatever": { "doc_id": <id> } } }`
    * you get the idea...

The point I’m trying to make here, and you will see it when I’ll cover different GraphQL clients, is that there is no standard, there are many, and there could be new standards soon.
That’s why Hive Router needs to be flexible enough to support all kinds of kinky shit.

We do care about performance, so relying on VRL expression for the extraction of the document id, on every request, is not really an option, at least not the one we should do and call it a day :)

### Apollo Client

[Persisted Documents in Apollo Client](./apollo-client.md)

This is what Apollo Client sends by default (when you configure it the way it is intended by Apollo team - according to docs)

```json
{
  "operationName":"ApolloCountries",
  "variables":{},
  "extensions":{
    "clientLibrary":{
      "name":"@apollo/client",
      "version":"4.1.6"
    },
    "persistedQuery":{
      "version":1,
      "sha256Hash":"9f9d50d29760468b4b4779822fa742270723d2b426a4dcfc93eb3d63d38fda87"
    }
  }
}
```

When you configure the clientAwarness feature (as I described in the linked canvas, you also get these headers

```
apollographql-client-name: example
apollographql-client-version: 1.0.0
```

### Relay Client

[Persisted Documents in Relay Client](./relay-client.md)

Relay case is interesting as it’s not enforcing any patterns. You can do whatever as you control the network layer.
What it showcases though, in the documentation and what is de facto a standard:

```graphql
{
  "doc_id":"0ebf7938810e26eb3938a5362307cf95",
  "operationName":"AppCountriesQuery",
  "variables":{}
}
```

### GraphQL HTTP Specification

Persisted Documents: GraphQL HTTP Specification does not really specify anything...
The draft or whatever the status of it is... accepts anything that may or may not contain : character.
When it does not you treat everything is the document id, but when it includes the semicolon, you get the xyz:<this-part>.

```
<algo>:<id>
sha256:7dba4bd717b41f10434822356a93c32b1fb4907b983e854300ad839f84cdcd6e

<id>
7dba4bd717b41f10434822356a93c32b1fb4907b983e854300ad839f84cdcd6e

x-<custom-value>:<id>
x-hive:7dba4bd717b41f10434822356a93c32b1fb4907b983e854300ad839f84cdcd6e
```

## Storage

It’s not only where we store but also what we store.

### Storage Format

Both GraphQL Codegen’s Client Preset and Relay’s Compiler produce a similar manifest file:

```json
{ "<hash>": "<text>" }
```

Apollo Client on the other hand, produces something more complex and different:

```json
{
  "format": "apollo-persisted-query-manifest",
  "version": 1,
  "operations": [
    {
      "id": "<hash>",
      "name": "<name>",
      "type": "query",
      "body": "query <name> { ... }"
    }
  ]
}
```

An alternative approach would be to store a single document per file, where the name of the file contains the document id. This is what Hive CDN does.

### Storage Space

Imo these are 4 popular ways of storing manifests or document-per-file files.

#### HTTP endpoint

I can imagine people writing their own registries (link to my talk) or doing some weird proxies. In order to support them we should give them a nice API.
Instead of creating some weird specification of how to fetch documents based on IDs, that is generic and easy to implement, and have ways to invalidate the cache etc, we should just point them to Plugin System and either expose a clean and easy to use API or rely on what the plugin system offers today.
A must here, is to create an example, at least in docs...

#### S3 compatible

Most of the time people will host the documents or manifests on S3/GCS/R1, basically S3 compatible storages.
I think we should natively support it in Hive Router and not point them to the plugin system.
There are many problems here:

* how to provide auth credentials given there are many different vendors
* different bucket names
* what’s stored may be different (document or manifest with documents?)

#### File

Mostly for development, as I can’t imagine people persisting a file next to the Router binary at scale...

* watch mode / polling is a must
*  I say we support one manifest file instead of a persisted/*.json globs or whole directory pointers ./persisted - one we have a need, we can add it. Let’s not overcomplicate it from day one as 99.9999% of cases it’s a single manifest file.
* support different manifest formats (should the config explicitly say what format the file is? Not sure as it has a DX cost and could be auto detected)

#### Hive CDN

What we should recommend and polish really well.

```
GET https://cdn.graphql-hive.com/artifacts/v1/:targetId/apps/:appName/:appVersion/:documentId
```

This is a bit tricky, because we not only rely on document id, but also app’s name and version.
The app’s name and version could be provided in many ways, but we should limit that to 2 options:

* client identification (request header for name, request header for version)
* hardcoded in document id (name~version~id)

Providing app’s name and version in headers gives a much better UX:

* Apollo Client has clientAwareness feature in which users provide name and version (apollographql-client-name/version headers)
* document id generated by “document extraction + persisting” tools is always a hash, so it’s natural to pass it as is to the http payload (Relay has params.id that could be used as {"doc_id": params.id } 
* Aligns better with Usage Reporting, tracing, metrics and logging

### Cache invalidation

What should happen when a document was resolved from the source?
When documents should be invalidated? When schema changes (naive but safe...).

Invalidation strategies per storage space:

*  File - invalidate when file changes
    * to increase the cache-hits ratio:
        * we could produce a checksum of the document text (minified)
        * store that checksum
        * compare the checksum and invalidate if gone or different
* HTTP - up to the plugin implementor to decide
* S3 - reuse cache headers sent back by the S3 storage or give an option to specify the TTL
* Hive - same rules as for S3 really


We need to not only cache the happy path (OK 200 with the file), but failures too.
The 404s should be cached for some time as well.
All configured with sensible defaults.

When it’s time to check whether the document is still active or not, we should serve the old one, but fetch the new one in the background, to swap later on. Basically the stale-while-revalidate pattern.

## Request Acceptance

I guess we could have a few level of strictness

* allow to execute only persisted documents
* allow to execute both persisted documents and regular requests

Additional logs for the migration period (from regular to persisted):

* ability to info log requests that are not persisted (full document body was sent)
* useful to detect rejected operations due to lack of document id

Apollo offers safelisting based on document’s string, not only based on the id, with ability to opt-in to require the id and reject non-id requests.


## Pipeline

When an http request with the document id hits the server:

1. document id is extracted from the request (Extraction)
2. document is resolved (Resolution)
3. document is injected into the graphql request
4. the graphql request continues to flow through the rest of the pipeline
    1. parse
    2. validate
    3. normalize
    4. plan
    5. execute

This adds latency to the first request (and identical-id requests that will be accepted during the time).

### Extraction

Gets info from HTTP request.
I think we should support these built-in extractors:

* URL path segment
* URL query param
* header
* JSON body field (path to get the id)
* Relay doc_id 
* Apollo’s extensions.persistedQuery.id

and optional custom extractor via plugin or VRL only as fallback.
These extractors could be defined in Hive Router as a list to configure precedence.

Dumb example code to what I mean:

```rust
struct DocumentRef<'a> {
  raw: &'a str,
  kind: DocumentRefKind,
}

enum DocumentRefKind<'a> {
  Opaque,
  Hash { algorithm: HashAlgorithm },
  Custom { prefix: &'a str },
}

struct ResolvedDocument<'a> {
  source: ResolvedDocumentSource,
  id: &'a str,
  text: Arc<str>,
  operation_name_hint: Option<&'a str>,
  metadata: DocumentMetadata,
}

struct ClientIdentity<'a> {
  name: Option<&'a str>,
  version: Option<&'a str>,
}

trait DocumentRefExtractor {
    fn extract<'a>(&self, request: &'a HttpRequestParts, body: Option<&'a [u8]>) -> ExtractionResult<'a>;
}

struct ExtractionResult<'a> {
    document_ref: Option<DocumentRef<'a>>,
    client_identity: ClientIdentity<'a>,
    metadata: ExtractionMetadata,
}
```

At the extraction level, we should enforce Request Acceptance.

### Resolution

Uses extracted info to load document text.
It should include a caching layer that resolves:

* found (doc text + metadata)
* not found
* error (reason)

The 404 cases should be treated differently than other errors. 5XX for example, should be retried, have shorter TTL.
Bult-in resolution impls:

* File manifest (format autodetected)
* S3 manifest (format autodetected)
* S3 object
* generic http (maybe?)
* Hive CDN

Dumb example code to what I mean:

```rust
trait PersistedDocumentResolver {
    async fn resolve<'a>(
        &self,
        reference: &DocumentRef<'a>,
        client_identity: &ClientIdentity<'a>,
        ctx: &ResolveContext,
    ) -> Result<ResolvedDocument<'a>, ResolveError>;
}
```

### Prewarming the caches

This is relatively cheap, because we multiplex the parsing, validation, normalization and planning to identical documents, happening at the same time. We do the work once.
We also don’t know all the persisted documents in advance (maybe we should?). We only know about those executed in the past.
When a new schema is loaded, the caches are busted.
This gives us an opportunity to avoid a spike in latencies of future requests!
We could prewarm the caches for recently used persisted operations, so next time the operation happens, it’s reusing the caches already.
It should be either opt-in or opt-out - to be decided.

If we knew all documents in advance, we could have prewarmed them all (or some, based on some factors like popularity), both on startup and on schema reload.

## Potential performance bottlenecks

* outbound HTTP request to Hive CDN for every fresh app name + app version + hash combination request
* big impact on latency on schema reloads (caches are nuked)
* big impact on latency on startup (fresh requests are uncached)
* http request to Hive CDN may take forever or be retired forever, when network issues occur (let’s have a sensible and configurable timeout)
* lots of http request resolved concurrently - we would have to put a limit on document resolver

## Observability

### Tracing

We should at least add the document id as the attribute. The client’s name and version is already attached to spans.

### Logs

We should at least add the document id as the attribute. We should also include client’s name and version.
Depending on Request Acceptance we should also inform user about rejections.

### Metrics

We should observe the two stages:

* document id extraction
* document resolution

Observe the duration, observe the hit/miss/error rates, when it makes sense.
Depending on Request Acceptance we should also inform user about rejections.


## Random stuff

* I think we should have a bypass behavior - like a header or something
* Research: the error codes and http status codes - basically how graphql clients handle the failures
* Ensure: when deserializing the request body, we don’t fail on extra/unknown fields
* Think: when allowing /graphql/1234, should it be treated as a persisted operation only or fallback to /graphql on invalid ids or something
