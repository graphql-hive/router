# Persisted Documents - Apollo ClientRepository with an example

Example available here: https://github.com/kamilkisiela/graphql-persisted-operations-example/tree/main/apps/apollo

Here’s what Apollo recommends: https://www.apollographql.com/docs/react/data/persisted-queries

## Manifest

Manifest is generated with https://www.npmjs.com/package/@apollo/generate-persisted-query-manifest

```bash
$ npx generate-persisted-query-manifest
```

A new file is created ./persisted-query-manifest.json with this content:

```json
{
  "format": "apollo-persisted-query-manifest",
  "version": 1,
  "operations": [
    {
      "id": "9f9d50d29760468b4b4779822fa742270723d2b426a4dcfc93eb3d63d38fda87",
      "name": "ApolloCountries",
      "type": "query",
      "body": "query ApolloCountries {\n  countries {\n    code\n    name\n    emoji\n    __typename\n  }\n}"
    }
  ]
}
```

## Client setup

Here’s the recommended (by Apollo docs) Apollo Client setup:

```js
import { ApolloClient, HttpLink, InMemoryCache } from "@apollo/client";
import { generatePersistedQueryIdsFromManifest } from "@apollo/persisted-query-lists";
import { PersistedQueryLink } from "@apollo/client/link/persisted-queries";

const persistedQueryLink = new PersistedQueryLink(
  generatePersistedQueryIdsFromManifest({
    loadManifest: () => import("../persisted-query-manifest.json"),
  }),
);

const httpLink = new HttpLink({
  uri: "http://localhost:4000/graphql",
});

export const apolloClient = new ApolloClient({
  cache: new InMemoryCache(),
  link: persistedQueryLink.concat(httpLink),
  clientAwareness: {
    name: "example",
    version: "1.0.0",
  },
});
``

Manifest is consumed by Apollo Client with https://www.npmjs.com/package/@apollo/persisted-query-lists.
We pass client’s name and version the way it’s intended in Apollo Client.

### Http Request

What Apollo Client sends to the server.

Body:
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

Headers:
```
apollographql-client-name: example
apollographql-client-version: 1.0.0
```

## Gateway

What the gateway knows:

* it’s Apollo format (use of extensions with persistedQuery field)
* document’s id
* operation’s name
* operation’s variables
* client library (name + version)
* app’s name and version


With this knowledge the gateway is capable of resolving a document from Hive CDN:

```
example~1.0.0~9f9d50d29760468b4b4779822fa742270723d2b426a4dcfc93eb3d63d38fda87
```
