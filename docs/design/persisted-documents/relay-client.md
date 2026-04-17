# Persisted Documents in Relay Client

Repository with an example: https://github.com/kamilkisiela/graphql-persisted-operations-example/tree/main/apps/relay

Here’s what Relay recommends: https://relay.dev/docs/guides/persisted-queries/

## Manifest

The manifest is generated with relay-compiler that is capable of watching code files and generating a new manifest on every file change.

An example `package.json`:

```json
{
  "scripts": {
    "persisted": "relay-compiler",
    "persisted:watch": "relay-compiler --watch"
  },
  "relay": {
    "src": "./src",
    "schema": "./schema.graphql",
    "language": "javascript",
    "artifactDirectory": "./src/__generated__",
    "persistConfig": {
    "file": "./persisted-queries.json",
    "algorithm": "MD5"
    }
  }
}
```

When `$ relay-compiler` runs it generates code in `src/__generated__`. That’s not unusual, that’s the regular workflow when using Relay.
The only difference is that the generated queries contain a unique id.
The compiler writes also a persisted-queries.json file with the manifest (mapping between ids and document texts).

## Client setup

It’s really up the the user, but the documentation says:

```js
import { Environment, Network, RecordSource, Store } from "relay-runtime"

async function fetchGraphQL(params, variables) {
  const response = await fetch("http://localhost:4000/graphql", {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify({
      doc_id: params.id,
      operationName: params.name,
      variables,
    }),
  })

  return response.json()
}

export const relayEnvironment = new Environment({
  network: Network.create(fetchGraphQL),
  store: new Store(new RecordSource()),
})
````

The relay’s compiler make sure to add id to every query in code, that’s why it’s available in params. No need to refer to the json file

### Http Request

What Relay Client sends to the server.

Body:
```json
{
  "doc_id":"0ebf7938810e26eb3938a5362307cf95",
  "operationName":"AppCountriesQuery",
  "variables":{}
}
```

Headers:
```
graphql-client-name: example
graphql-client-version: v1.0.0
```

## Gateway

What the gateway knows:

* it’s Relay format (doc_id)
* document’s id
* operation’s name
* operation’s variables
* app’s name and version


With this knowledge the gateway is capable of resolving a document from Hive CDN:

```
example~1.0.0~0ebf7938810e26eb3938a5362307cf95
```
