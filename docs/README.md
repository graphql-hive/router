# HiveRouterConfig

**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**http**](#http)|`object`|Configuration for the HTTP server/listener.<br/>Default: `{"host":"0.0.0.0","port":4000}`<br/>||
|[**jwt**](#jwt)|`object`, `null`|Configuration for JWT authentication plugin.<br/>|yes|
|[**log**](#log)|`object`|The router logger configuration.<br/>Default: `{"filter":null,"format":"json","level":"info"}`<br/>||
|[**query\_planner**](#query_planner)|`object`|Query planning configuration.<br/>Default: `{"allow_expose":false,"timeout":"10s"}`<br/>||
|[**supergraph**](#supergraph)|`object`|Configuration for the Federation supergraph source. By default, the router will use a local file-based supergraph source (`./supergraph.graphql`).<br/>||
|[**traffic\_shaping**](#traffic_shaping)|`object`|Configuration for the traffic-shaper executor. Use these configurations to control how requests are being executed to subgraphs.<br/>Default: `{"dedupe_enabled":true,"dedupe_fingerprint_headers":["authorization"],"max_connections_per_host":100,"pool_idle_timeout_seconds":50}`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
http:
  host: 0.0.0.0
  port: 4000
log:
  filter: null
  format: json
  level: info
query_planner:
  allow_expose: false
  timeout: 10s
supergraph: {}
traffic_shaping:
  dedupe_enabled: true
  dedupe_fingerprint_headers:
    - authorization
  max_connections_per_host: 100
  pool_idle_timeout_seconds: 50

```

<a name="http"></a>
## http: object

Configuration for the HTTP server/listener.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**host**|`string`|The host address to bind the HTTP server to.<br/>Default: `"0.0.0.0"`<br/>||
|**port**|`integer`|The port to bind the HTTP server to.<br/><br/>If you are running the router inside a Docker container, please ensure that the port is exposed correctly using `-p <host_port>:<container_port>` flag.<br/>Default: `4000`<br/>Format: `"uint16"`<br/>Minimum: `0`<br/>Maximum: `65535`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
host: 0.0.0.0
port: 4000

```

<a name="jwt"></a>
## jwt: object,null

Configuration for JWT authentication plugin.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**allowed\_algorithms**](#jwtallowed_algorithms)|`string[]`|List of allowed algorithms for verifying the JWT signature.<br/>Default: `"HS256"`, `"HS384"`, `"HS512"`, `"RS256"`, `"RS384"`, `"RS512"`, `"ES256"`, `"ES384"`, `"PS256"`, `"PS384"`, `"PS512"`, `"EdDSA"`<br/>|no|
|[**audiences**](#jwtaudiences)|`string[]`|The list of [JWT audiences](https://tools.ietf.org/html/rfc7519#section-4.1.3) are allowed to access.<br/>|no|
|**forward\_claims\_to\_upstream\_header**|`string`, `null`|Forward the JWT claims to the upstream service in the specified header.<br/>|no|
|**forward\_token\_to\_upstream\_header**|`string`, `null`|Forward the JWT token to the upstream service in the specified header.<br/>|no|
|[**issuers**](#jwtissuers)|`string[]`|Specify the [principal](https://tools.ietf.org/html/rfc7519#section-4.1.1) that issued the JWT, usually a URL or an email address.<br/>|no|
|[**jwks\_providers**](#jwtjwks_providers)|`array`|A list of JWKS providers to use for verifying the JWT signature.<br/>|yes|
|[**lookup\_locations**](#jwtlookup_locations)|`array`|A list of locations to look up for the JWT token in the incoming HTTP request.<br/>Default: `{"name":"Authorization","prefix":"Bearer","source":"header"}`<br/>|no|
|**require\_authentication**|`boolean`, `null`|If set to `true`, the entire request will be rejected if the JWT token is not present in the request.<br/>|no|

**Additional Properties:** not allowed  
<a name="jwtallowed_algorithms"></a>
### jwt\.allowed\_algorithms\[\]: array,null

List of allowed algorithms for verifying the JWT signature.
If not specified, the default list of all supported algorithms in [`jsonwebtoken` crate](https://crates.io/crates/jsonwebtoken) are used.


**Items**

**Item Type:** `string`  
**Example**

```yaml
- HS256
- HS384
- HS512
- RS256
- RS384
- RS512
- ES256
- ES384
- PS256
- PS384
- PS512
- EdDSA

```

<a name="jwtaudiences"></a>
### jwt\.audiences\[\]: array,null

The list of [JWT audiences](https://tools.ietf.org/html/rfc7519#section-4.1.3) are allowed to access.
If this field is set, the token's `aud` field must be one of the values in this list, otherwise the token's `aud` field is not checked.


**Items**

**Item Type:** `string`  
<a name="jwtissuers"></a>
### jwt\.issuers\[\]: array,null

Specify the [principal](https://tools.ietf.org/html/rfc7519#section-4.1.1) that issued the JWT, usually a URL or an email address.
If specified, it has to match the `iss` field in JWT, otherwise the token's `iss` field is not checked.


**Items**

**Item Type:** `string`  
<a name="jwtjwks_providers"></a>
### jwt\.jwks\_providers\[\]: array

A list of JWKS providers to use for verifying the JWT signature.
Can be either a path to a local JSON of the file-system, or a URL to a remote JWKS provider.


**Items**

   
**Option 1 (alternative):** 
A local file on the file-system. This file will be read once on startup and cached.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**path**|`string`|A path to a local file on the file-system. Relative to the location of the root configuration file.<br/>Format: `"path"`<br/>|yes|
|**source**|`string`|Constant Value: `"file"`<br/>|yes|


   
**Option 2 (alternative):** 
A remote JWKS provider. The JWKS will be fetched via HTTP/HTTPS and cached.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**polling\_interval**|`string`|How often the JWKS should be polled for updates.<br/>Default: `"10m"`<br/>|no|
|**prefetch**|`boolean`, `null`|If set to `true`, the JWKS will be fetched on startup and cached. In case of invalid JWKS, the error will be ignored and the plugin will try to fetch again when server receives the first request.<br/>If set to `false`, the JWKS will be fetched on-demand, when the first request comes in.<br/>|no|
|**source**|`string`|Constant Value: `"remote"`<br/>|yes|
|**url**|`string`|The URL to fetch the JWKS key set from, via HTTP/HTTPS.<br/>|yes|

**Example**

```yaml
polling_interval: 10m

```


<a name="jwtlookup_locations"></a>
### jwt\.lookup\_locations\[\]: array

A list of locations to look up for the JWT token in the incoming HTTP request.
The first one that is found will be used.


**Items**

   
**Option 1 (alternative):** 
**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**name**|`string`||yes|
|**prefix**|`string`, `null`||no|
|**source**|`string`|Constant Value: `"header"`<br/>|yes|


   
**Option 2 (alternative):** 
**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**name**|`string`||yes|
|**source**|`string`|Constant Value: `"cookies"`<br/>|yes|


**Example**

```yaml
- name: Authorization
  prefix: Bearer
  source: header

```

<a name="log"></a>
## log: object

The router logger configuration.

The router is configured to be mostly silent (`info`) level, and will print only important messages, warnings, and errors.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**filter**|`string`, `null`|||
|**format**|`string`|Default: `"json"`<br/>Enum: `"pretty-tree"`, `"pretty-compact"`, `"json"`<br/>||
|**level**|`string`|Default: `"info"`<br/>Enum: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
filter: null
format: json
level: info

```

<a name="query_planner"></a>
## query\_planner: object

Query planning configuration.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**allow\_expose**|`boolean`|A flag to allow exposing the query plan in the response.<br/>When set to `true` and an incoming request has a `hive-expose-query-plan: true` header, the query plan will be exposed in the response, as part of `extensions`.<br/>Default: `false`<br/>||
|**timeout**|`string`|The maximum time for the query planner to create an execution plan.<br/>This acts as a safeguard against overly complex or malicious queries that could degrade server performance.<br/>When the timeout is reached, the planning process is cancelled.<br/><br/>Default: 10s.<br/>Default: `"10s"`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
allow_expose: false
timeout: 10s

```

<a name="supergraph"></a>
## supergraph: object

Configuration for the Federation supergraph source. By default, the router will use a local file-based supergraph source (`./supergraph.graphql`).
Each source has a different set of configuration, depending on the source type.


   
**Option 1 (alternative):** 
Loads a supergraph from the filesystem.
The path can be either absolute or relative to the router's working directory.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**path**|`string`|Format: `"path"`<br/>|yes|
|**source**|`string`|Constant Value: `"file"`<br/>|yes|

**Additional Properties:** not allowed  

<a name="traffic_shaping"></a>
## traffic\_shaping: object

Configuration for the traffic-shaper executor. Use these configurations to control how requests are being executed to subgraphs.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**dedupe\_enabled**|`boolean`|Enables/disables request deduplication to subgraphs.<br/><br/>When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will<br/>be deduplicated by sharing the response of other in-flight requests.<br/>Default: `true`<br/>||
|[**dedupe\_fingerprint\_headers**](#traffic_shapingdedupe_fingerprint_headers)|`string[]`|A list of headers that should be used to fingerprint requests for deduplication.<br/>Default: `"authorization"`<br/>||
|**max\_connections\_per\_host**|`integer`|Limits the concurrent amount of requests/connections per host/subgraph.<br/>Default: `100`<br/>Format: `"uint"`<br/>Minimum: `0`<br/>||
|**pool\_idle\_timeout\_seconds**|`integer`|Timeout for idle sockets being kept-alive.<br/>Default: `50`<br/>Format: `"uint64"`<br/>Minimum: `0`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
dedupe_enabled: true
dedupe_fingerprint_headers:
  - authorization
max_connections_per_host: 100
pool_idle_timeout_seconds: 50

```

<a name="traffic_shapingdedupe_fingerprint_headers"></a>
### traffic\_shaping\.dedupe\_fingerprint\_headers\[\]: array

A list of headers that should be used to fingerprint requests for deduplication.

If not provided, the default is to use the "authorization" header only.


**Items**

**Item Type:** `string`  
**Example**

```yaml
- authorization

```


