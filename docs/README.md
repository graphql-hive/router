# HiveRouterConfig

**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**cors**](#cors)|`object`|Configuration for CORS (Cross-Origin Resource Sharing).<br/>Default: `{"allow_any_origin":false,"allow_credentials":false,"enabled":false,"policies":[]}`<br/>|yes|
|[**csrf**](#csrf)|`object`|Configuration for CSRF prevention.<br/>Default: `{"enabled":false,"required_headers":[]}`<br/>||
|[**graphiql**](#graphiql)|`object`|Configuration for the GraphiQL interface.<br/>Default: `{"enabled":true}`<br/>||
|[**headers**](#headers)|`object`|Configuration for the headers.<br/>Default: `{}`<br/>||
|[**http**](#http)|`object`|Configuration for the HTTP server/listener.<br/>Default: `{"host":"0.0.0.0","port":4000}`<br/>||
|[**jwt**](#jwt)|`object`|Configuration for JWT authentication plugin.<br/>|yes|
|[**log**](#log)|`object`|The router logger configuration.<br/>Default: `{"filter":null,"format":"json","level":"info"}`<br/>||
|[**override\_labels**](#override_labels)|`object`|Configuration for overriding labels.<br/>||
|[**override\_subgraph\_urls**](#override_subgraph_urls)|`object`|Configuration for overriding subgraph URLs.<br/>Default: `{}`<br/>||
|[**query\_planner**](#query_planner)|`object`|Query planning configuration.<br/>Default: `{"allow_expose":false,"timeout":"10s"}`<br/>||
|[**supergraph**](#supergraph)|`object`|Configuration for the Federation supergraph source. By default, the router will use a local file-based supergraph source (`./supergraph.graphql`).<br/>||
|[**traffic\_shaping**](#traffic_shaping)|`object`|Configuration for the traffic-shaping of the executor. Use these configurations to control how requests are being executed to subgraphs.<br/>Default: `{"dedupe_enabled":true,"max_connections_per_host":100,"pool_idle_timeout_seconds":50}`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
cors:
  allow_any_origin: false
  allow_credentials: false
  enabled: true
  max_age: 120
  methods:
    - GET
    - POST
    - OPTIONS
  policies:
    - origins:
        - https://example.com
        - https://another.com
csrf:
  enabled: true
  required_headers:
    - x-csrf-token
graphiql:
  enabled: true
headers:
  all:
    request:
      - propagate:
          named: Authorization
      - remove:
          matching: ^x-legacy-.*
      - insert:
          name: x-router
          value: hive-router
  subgraphs:
    accounts:
      request:
        - propagate:
            default: unknown
            named: x-tenant-id
            rename: x-acct-tenant
http:
  host: 0.0.0.0
  port: 4000
jwt:
  allowed_algorithms:
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
  enabled: false
  forward_claims_to_upstream_extensions:
    enabled: false
    field_name: jwt
  lookup_locations:
    - name: authorization
      prefix: Bearer
      source: header
log:
  filter: null
  format: json
  level: info
override_labels: {}
override_subgraph_urls:
  accounts:
    url: https://accounts.example.com/graphql
  products:
    url:
      expression: |2-

                if .request.headers."x-region" == "us-east" {
                    "https://products-us-east.example.com/graphql"
                } else if .request.headers."x-region" == "eu-west" {
                    "https://products-eu-west.example.com/graphql"
                } else {
                  .original_url
                }
            
query_planner:
  allow_expose: false
  timeout: 10s
supergraph: {}
traffic_shaping:
  dedupe_enabled: true
  max_connections_per_host: 100
  pool_idle_timeout_seconds: 50

```

<a name="cors"></a>
## cors: object

Configuration for CORS (Cross-Origin Resource Sharing).


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**allow\_any\_origin**|`boolean`|Set to true to allow any origin. If true, the `origins` and `match_origin` fields are ignored.<br/>Default: `false`<br/>|no|
|**allow\_credentials**|`boolean`|Set to true to allow credentials (cookies, authorization headers, or TLS client certificates) in cross-origin requests.<br/>This will set the `Access-Control-Allow-Credentials` header to `true`.<br/>Default: `false`<br/>|no|
|[**allow\_headers**](#corsallow_headers)|`string[]`|List of headers that the server allows the client to send in a cross-origin request.<br/>|no|
|**enabled**|`boolean`|Default: `false`<br/>|no|
|[**expose\_headers**](#corsexpose_headers)|`string[]`|List of headers that the client is allowed to access from the response.<br/>|no|
|**max\_age**|`integer`, `null`|The maximum time (in seconds) that the results of a preflight request can be cached by the client.<br/>This will set the `Access-Control-Max-Age` header.<br/>If not set, the browser will not cache the preflight response.<br/>Example: 86400 (24 hours)<br/>Format: `"uint64"`<br/>Minimum: `0`<br/>|no|
|[**methods**](#corsmethods)|`string[]`|List of methods that the server allows for cross-origin requests.<br/>|no|
|[**policies**](#corspolicies)|`object[]`|List of CORS policies. The first policy that matches the request origin will be applied.<br/>|yes|

**Example**

```yaml
allow_any_origin: false
allow_credentials: false
enabled: true
max_age: 120
methods:
  - GET
  - POST
  - OPTIONS
policies:
  - origins:
      - https://example.com
      - https://another.com

```

**Example**

```yaml
allow_any_origin: true
allow_credentials: false
enabled: true
policies: []

```

<a name="corsallow_headers"></a>
### cors\.allow\_headers\[\]: array,null

List of headers that the server allows the client to send in a cross-origin request.
This will set the `Access-Control-Allow-Headers` header.
If not set, the server will reflect the headers specified in the `Access-Control-Request-Headers` request header.
Example: ["Content-Type", "Authorization"]


**Items**

**Item Type:** `string`  
<a name="corsexpose_headers"></a>
### cors\.expose\_headers\[\]: array,null

List of headers that the client is allowed to access from the response.
This will set the `Access-Control-Expose-Headers` header.
If not set, no additional headers are exposed to the client.
Example: ["X-Custom-Header", "X-Another-Header"]


**Items**

**Item Type:** `string`  
<a name="corsmethods"></a>
### cors\.methods\[\]: array,null

List of methods that the server allows for cross-origin requests.
This will set the `Access-Control-Allow-Methods` header.
If not set, the server will reflect the method specified in the `Access-Control-Request-Method` request header.
Example: ["GET", "POST", "OPTIONS"]


**Items**

**Item Type:** `string`  
<a name="corspolicies"></a>
### cors\.policies\[\]: array

List of CORS policies. The first policy that matches the request origin will be applied.
If no policies match, the request will be rejected.
If `allow_any_origin` is true, this field is ignored.
This allows you to define different CORS settings for different origins.
For example, you might want to allow credentials for some origins but not others.
If multiple policies match, the first one in the list will be applied.

Example:
```yaml
allow_credentials: false
policies:
  - match_origin: ["^https://.*\.credentials-example\.com$"]
    allow_credentials: true
  - match_origin: ["^https://.*\.example\.com$"]
```

In this example, requests from any subdomain of `credentials-example.com` will be allowed to include credentials,
while requests from any subdomain of `example.com` will not be allowed to include credentials.
Requests from origins not matching either pattern will be rejected.

## Policy Inheritance Rules

Each policy defined in the `policies` array can provide its own CORS settings.
If a setting is not specified within a policy, the corresponding global CORS setting is used as a fallback.

Here's a breakdown of how inheritance works for each field:

- `allow_credentials` and `max_age`: If a policy omits a value for these settings,
  it automatically uses the value from the global configuration.
- `allow_headers` and `expose_headers`: A policy's behavior for these header lists depends on the value provided:
  - If a list with specific headers is provided (e.g., `["Content-Type"]`), it completely overrides the global list.
  - If an empty list (`[]`) is provided, the policy will inherit the headers from the global configuration.
- `methods`: This setting has three distinct states for inheritance:
  - If `methods` is not specified at all (`null`), the policy inherits the global methods.
  - If an empty list (`[]`) is provided, no methods are allowed for that policy.
  - If the list contains specific methods (e.g., `["GET", "POST"]`), only those methods are used, overriding the global list.


**Items**

**Item Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**allow\_credentials**|`boolean`, `null`|Set to true to allow credentials (cookies, authorization headers, or TLS client certificates) in cross-origin requests.<br/>This will set the `Access-Control-Allow-Credentials` header to `true`.<br/>||
|[**allow\_headers**](#corspoliciesallow_headers)|`string[]`|List of headers that the server allows the client to send in a cross-origin request.<br/>||
|[**expose\_headers**](#corspoliciesexpose_headers)|`string[]`|List of headers that the client is allowed to access from the response.<br/>||
|[**match\_origin**](#corspoliciesmatch_origin)|`string[]`|List of regex patterns to match allowed origins. If `allow_any_origin` is true, this field is ignored.<br/>||
|**max\_age**|`integer`, `null`|The maximum time (in seconds) that the results of a preflight request can be cached by the client.<br/>This will set the `Access-Control-Max-Age` header.<br/>If not set, the browser will not cache the preflight response.<br/>Example: 86400 (24 hours)<br/>Format: `"uint64"`<br/>Minimum: `0`<br/>||
|[**methods**](#corspoliciesmethods)|`string[]`|List of methods that the server allows for cross-origin requests.<br/>||
|[**origins**](#corspoliciesorigins)|`string[]`|List of allowed origins. If `allow_any_origin` is true, this field is ignored.<br/>||

**Example**

```yaml
- {}

```

<a name="corspoliciesallow_headers"></a>
#### cors\.policies\[\]\.allow\_headers\[\]: array,null

List of headers that the server allows the client to send in a cross-origin request.
This will set the `Access-Control-Allow-Headers` header.
If not set, the server will reflect the headers specified in the `Access-Control-Request-Headers` request header.
Example: ["Content-Type", "Authorization"]


**Items**

**Item Type:** `string`  
<a name="corspoliciesexpose_headers"></a>
#### cors\.policies\[\]\.expose\_headers\[\]: array,null

List of headers that the client is allowed to access from the response.
This will set the `Access-Control-Expose-Headers` header.
If not set, no additional headers are exposed to the client.
Example: ["X-Custom-Header", "X-Another-Header"]


**Items**

**Item Type:** `string`  
<a name="corspoliciesmatch_origin"></a>
#### cors\.policies\[\]\.match\_origin\[\]: array,null

List of regex patterns to match allowed origins. If `allow_any_origin` is true, this field is ignored.
If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
Each pattern should be a valid regex.
Example: "^https://.*\.example\.com$", "^http://localhost:\d+$"


**Items**

**Item Type:** `string`  
<a name="corspoliciesmethods"></a>
#### cors\.policies\[\]\.methods\[\]: array,null

List of methods that the server allows for cross-origin requests.
This will set the `Access-Control-Allow-Methods` header.
If not set, the server will reflect the method specified in the `Access-Control-Request-Method` request header.
Example: ["GET", "POST", "OPTIONS"]


**Items**

**Item Type:** `string`  
<a name="corspoliciesorigins"></a>
#### cors\.policies\[\]\.origins\[\]: array,null

List of allowed origins. If `allow_any_origin` is true, this field is ignored.
If both `origins` and `match_origin` are set, the request origin must match one of the values in either list to be allowed.
An origin is a combination of scheme, host, and port (if specified).
Example: "https://example.com", "http://localhost:3000"


**Items**

**Item Type:** `string`  
<a name="csrf"></a>
## csrf: object

Configuration for CSRF prevention.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**enabled**|`boolean`|Enables CSRF prevention.<br/><br/>By enabling CSRF prevention, the router will check for the presence of specific headers in incoming requests to the `/graphql` endpoint.<br/>If the required headers are not present, the router will reject the request with a `403 Forbidden` response.<br/>This triggers the preflight checks in browsers, preventing the request from being sent.<br/>So you can ensure that only requests from trusted origins are processed.<br/><br/>When CSRF prevention is enabled, the router only executes operations if one of the following conditions is true;<br/><br/>- The incoming request includes a `Content-Type` header other than a value of<br/>  - `text/plain`<br/>  - `application/x-www-form-urlencoded`<br/>  - `multipart/form-data`<br/><br/>- The incoming request includes at least one of the headers specified in the `required_headers` configuration.<br/>Default: `true`<br/>||
|[**required\_headers**](#csrfrequired_headers)|`string[]`|A list of required header names for CSRF protection.<br/>Default: <br/>||

**Example**

```yaml
enabled: true
required_headers:
  - x-csrf-token

```

<a name="csrfrequired_headers"></a>
### csrf\.required\_headers\[\]: array

A list of required header names for CSRF protection.

Header names are case-insensitive.


**Items**


A valid HTTP header name, according to RFC 7230.

**Item Type:** `string`  
**Item Pattern:** `^[A-Za-z0-9!#$%&'*+\-.^_\`\|~]+$`  
<a name="graphiql"></a>
## graphiql: object

Configuration for the GraphiQL interface.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**enabled**|`boolean`|Enables/disables the GraphiQL interface. By default, the GraphiQL interface is enabled.<br/><br/>You can override this setting by setting the `GRAPHIQL_ENABLED` environment variable to `true` or `false`.<br/>Default: `true`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
enabled: true

```

<a name="headers"></a>
## headers: object

Configuration for the headers.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**all**](#headersall)|`object`, `null`|Rules applied to all subgraphs (global defaults).<br/>||
|[**subgraphs**](#headerssubgraphs)|`object`, `null`|Rules applied to individual subgraphs.<br/>||

**Example**

```yaml
all:
  request:
    - propagate:
        named: Authorization
    - remove:
        matching: ^x-legacy-.*
    - insert:
        name: x-router
        value: hive-router
subgraphs:
  accounts:
    request:
      - propagate:
          default: unknown
          named: x-tenant-id
          rename: x-acct-tenant

```

<a name="headersall"></a>
### headers\.all: object,null

Rules applied to all subgraphs (global defaults).


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**request**](#headersallrequest)|`array`|Rules that shape the **request** sent from the router to subgraphs.<br/>||
|[**response**](#headersallresponse)|`array`|Rules that shape the **response** sent from the router back to the client.<br/>||

<a name="headersallrequest"></a>
#### headers\.all\.request\[\]: array,null

Rules that shape the **request** sent from the router to subgraphs.


**Items**


Request-header rules (applied before sending to a subgraph).

   
**Option 1 (alternative):** 
Forward headers from the client request into the subgraph request.

- If `rename` is set, the header is forwarded under the new name.
- If **none** of the matched headers exist, `default` is used (when provided).

**Order matters:** You can propagate first and then `remove` or `insert`
to refine the final output.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**propagate**](#option1propagate)|`object`|Propagate headers from the client request to subgraph requests.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
propagate: {}

```


   
**Option 2 (alternative):** 
Remove headers before sending the request to a subgraph.

Useful to drop sensitive or irrelevant headers, or to undo a previous
`propagate`/`insert`.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**remove**](#option2remove)|`object`|Remove headers matched by the specification.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
remove: {}

```


   
**Option 3 (alternative):** 
Add or overwrite a header with a static value.

- For **normal** headers: replaces any existing value.
- For **never-join** headers (e.g. `set-cookie`): **appends** another
  occurrence (multiple lines), never comma-joins.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**insert**](#option3insert)|`object`|Insert a header with a static value.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
insert: {}

```


<a name="option1propagate"></a>
## Option 1: propagate: object

Propagate headers from the client request to subgraph requests.

**Behavior**
- If `rename` is provided, forwarded under that name.
- If **none** of the matched headers are present, `default` (when present)
  is used under `rename` (if set) or the **first** `named` header.

### Examples
```yaml
# Forward a specific header, but rename it per subgraph
propagate:
  named: x-tenant-id
  rename: x-acct-tenant

# Forward all x- headers except legacy ones
propagate:
  matching: "^x-.*"
  exclude: ["^x-legacy-.*"]

# If Authorization is missing, inject a default token for this subgraph
propagate:
  named: Authorization
  default: "Bearer test-token"
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**default**|`string`, `null`|If the header is missing, set a default value.<br/>Applied only when **none** of the matched headers exist.<br/>||
|[**exclude**](#option1propagateexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||
|**rename**|`string`, `null`|Optionally rename the header when forwarding.<br/>||

<a name="option1propagateexclude"></a>
### Option 1: propagate\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option2remove"></a>
## Option 2: remove: object

Remove headers matched by the specification.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**exclude**](#option2removeexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||

<a name="option2removeexclude"></a>
### Option 2: remove\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option3insert"></a>
## Option 3: insert: object

Insert a header with a static value.

### Examples
```yaml
- insert:
    name: x-env
    value: prod
```

```yaml
- insert:
    name: set-cookie
    value: "a=1; Path=/"
# If another Set-Cookie exists, this creates another header line (never joined)
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**name**|`string`|Header name to insert or overwrite (case-insensitive).<br/>|yes|

   
**Option 1 (optional):** 
Static value provided in the config.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**value**|`string`||yes|


   
**Option 2 (optional):** 
A dynamic value computed by a VRL expression.

This allows you to generate header values based on the incoming request,
subgraph name, and (for response rules) subgraph response headers.
The expression has access to a context object with `.request`, `.subgraph`,
and `.response` fields.

For more information on the available functions and syntax, see the
[VRL documentation](https://vrl.dev/).

### Example
```yaml
# Insert a header with a value derived from another header.
- insert:
    name: x-auth-scheme
    expression: 'split(.request.headers.authorization, " ")[0] ?? "none"'
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**expression**|`string`|The VRL expression string.<br/>|yes|


<a name="headersallresponse"></a>
#### headers\.all\.response\[\]: array,null

Rules that shape the **response** sent from the router back to the client.


**Items**


Response-header rules (applied before sending back to the client).

   
**Option 1 (alternative):** 
Forward headers from subgraph responses into the final client response.

- If multiple subgraphs provide the same header, `algorithm` controls
  how values are merged.
- If **no** subgraph provides a matching header, `default` is used (when provided).
- If `rename` is set, the header is returned under the new name.

**Never-join headers** (e.g. `set-cookie`) are never comma-joined:
multiple values are returned as separate header fields regardless of `algorithm`.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**propagate**](#option1propagate)|`object`|Propagate headers from subgraph responses to the final client response.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
propagate: {}

```


   
**Option 2 (alternative):** 
Remove headers before sending the response to the client.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**remove**](#option2remove)|`object`|Remove headers matched by the specification.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
remove: {}

```


   
**Option 3 (alternative):** 
Add or overwrite a header in the response to the client.

For never-join headers, appends another occurrence (multiple lines).


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**insert**](#option3insert)|`object`|Insert a header with a static value.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
insert: {}

```


<a name="option1propagate"></a>
## Option 1: propagate: object

Propagate headers from subgraph responses to the final client response.

**Behavior**
- If multiple subgraphs return the header, values are merged using `algorithm`.
  Never-join headers are **never** comma-joined.
- If **no** subgraph returns a match, `default` (if set) is emitted.
- If `rename` is set, the outgoing header uses the new name.

### Examples
```yaml
# Forward Cache-Control from whichever subgraph supplies it (last wins)
propagate:
  named: Cache-Control
  algorithm: last

# Combine list-valued headers
propagate:
  named: vary
  algorithm: append

# Ensure a fallback header is always present
propagate:
  named: x-backend
  algorithm: append
  default: unknown
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**algorithm**||How to merge values across multiple subgraph responses.<br/>|yes|
|**default**|`string`, `null`|If no subgraph returns the header, set this default value.<br/>|no|
|[**exclude**](#option1propagateexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>|no|
|**matching**||Match headers by regex pattern(s) (OR).<br/>|no|
|**named**||Match headers by exact name (OR).<br/>|no|
|**rename**|`string`, `null`|Optionally rename the header when returning it to the client.<br/>|no|

<a name="option1propagateexclude"></a>
### Option 1: propagate\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option2remove"></a>
## Option 2: remove: object

Remove headers matched by the specification.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**exclude**](#option2removeexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||

<a name="option2removeexclude"></a>
### Option 2: remove\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option3insert"></a>
## Option 3: insert: object

Insert a header with a static value.

### Examples
```yaml
- insert:
    name: x-env
    value: prod
```

```yaml
- insert:
    name: set-cookie
    value: "a=1; Path=/"
# If another Set-Cookie exists, this creates another header line (never joined)
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**algorithm**||How to merge values across multiple subgraph responses.<br/>Default: `Last` (overwrite).<br/>|no|
|**name**|`string`|Header name to insert or overwrite (case-insensitive).<br/>|yes|

   
**Option 1 (optional):** 
Static value provided in the config.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**value**|`string`||yes|


   
**Option 2 (optional):** 
A dynamic value computed by a VRL expression.

This allows you to generate header values based on the incoming request,
subgraph name, and (for response rules) subgraph response headers.
The expression has access to a context object with `.request`, `.subgraph`,
and `.response` fields.

For more information on the available functions and syntax, see the
[VRL documentation](https://vrl.dev/).

### Example
```yaml
# Insert a header with a value derived from another header.
- insert:
    name: x-auth-scheme
    expression: 'split(.request.headers.authorization, " ")[0] ?? "none"'
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**expression**|`string`|The VRL expression string.<br/>|yes|


<a name="headerssubgraphs"></a>
### headers\.subgraphs: object,null

Rules applied to individual subgraphs.
Keys are subgraph names as defined in the supergraph schema.

**Precedence:** These are applied **after** `all`, and therefore can
override the result of global rules for that subgraph.


**Additional Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**Additional Properties**](#headerssubgraphsadditionalproperties)|`object`|Rules for a single scope (global or per-subgraph).<br/>||

<a name="headerssubgraphsadditionalproperties"></a>
#### headers\.subgraphs\.additionalProperties: object

Rules for a single scope (global or per-subgraph).

You can specify independent rule lists for **request** (to subgraphs)
and **response** (to clients). Within each list, rules are applied in order.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**request**](#headerssubgraphsadditionalpropertiesrequest)|`array`|Rules that shape the **request** sent from the router to subgraphs.<br/>||
|[**response**](#headerssubgraphsadditionalpropertiesresponse)|`array`|Rules that shape the **response** sent from the router back to the client.<br/>||

<a name="headerssubgraphsadditionalpropertiesrequest"></a>
##### headers\.subgraphs\.additionalProperties\.request\[\]: array,null

Rules that shape the **request** sent from the router to subgraphs.


**Items**


Request-header rules (applied before sending to a subgraph).

   
**Option 1 (alternative):** 
Forward headers from the client request into the subgraph request.

- If `rename` is set, the header is forwarded under the new name.
- If **none** of the matched headers exist, `default` is used (when provided).

**Order matters:** You can propagate first and then `remove` or `insert`
to refine the final output.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**propagate**](#option1propagate)|`object`|Propagate headers from the client request to subgraph requests.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
propagate: {}

```


   
**Option 2 (alternative):** 
Remove headers before sending the request to a subgraph.

Useful to drop sensitive or irrelevant headers, or to undo a previous
`propagate`/`insert`.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**remove**](#option2remove)|`object`|Remove headers matched by the specification.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
remove: {}

```


   
**Option 3 (alternative):** 
Add or overwrite a header with a static value.

- For **normal** headers: replaces any existing value.
- For **never-join** headers (e.g. `set-cookie`): **appends** another
  occurrence (multiple lines), never comma-joins.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**insert**](#option3insert)|`object`|Insert a header with a static value.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
insert: {}

```


<a name="option1propagate"></a>
## Option 1: propagate: object

Propagate headers from the client request to subgraph requests.

**Behavior**
- If `rename` is provided, forwarded under that name.
- If **none** of the matched headers are present, `default` (when present)
  is used under `rename` (if set) or the **first** `named` header.

### Examples
```yaml
# Forward a specific header, but rename it per subgraph
propagate:
  named: x-tenant-id
  rename: x-acct-tenant

# Forward all x- headers except legacy ones
propagate:
  matching: "^x-.*"
  exclude: ["^x-legacy-.*"]

# If Authorization is missing, inject a default token for this subgraph
propagate:
  named: Authorization
  default: "Bearer test-token"
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**default**|`string`, `null`|If the header is missing, set a default value.<br/>Applied only when **none** of the matched headers exist.<br/>||
|[**exclude**](#option1propagateexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||
|**rename**|`string`, `null`|Optionally rename the header when forwarding.<br/>||

<a name="option1propagateexclude"></a>
### Option 1: propagate\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option2remove"></a>
## Option 2: remove: object

Remove headers matched by the specification.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**exclude**](#option2removeexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||

<a name="option2removeexclude"></a>
### Option 2: remove\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option3insert"></a>
## Option 3: insert: object

Insert a header with a static value.

### Examples
```yaml
- insert:
    name: x-env
    value: prod
```

```yaml
- insert:
    name: set-cookie
    value: "a=1; Path=/"
# If another Set-Cookie exists, this creates another header line (never joined)
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**name**|`string`|Header name to insert or overwrite (case-insensitive).<br/>|yes|

   
**Option 1 (optional):** 
Static value provided in the config.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**value**|`string`||yes|


   
**Option 2 (optional):** 
A dynamic value computed by a VRL expression.

This allows you to generate header values based on the incoming request,
subgraph name, and (for response rules) subgraph response headers.
The expression has access to a context object with `.request`, `.subgraph`,
and `.response` fields.

For more information on the available functions and syntax, see the
[VRL documentation](https://vrl.dev/).

### Example
```yaml
# Insert a header with a value derived from another header.
- insert:
    name: x-auth-scheme
    expression: 'split(.request.headers.authorization, " ")[0] ?? "none"'
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**expression**|`string`|The VRL expression string.<br/>|yes|


<a name="headerssubgraphsadditionalpropertiesresponse"></a>
##### headers\.subgraphs\.additionalProperties\.response\[\]: array,null

Rules that shape the **response** sent from the router back to the client.


**Items**


Response-header rules (applied before sending back to the client).

   
**Option 1 (alternative):** 
Forward headers from subgraph responses into the final client response.

- If multiple subgraphs provide the same header, `algorithm` controls
  how values are merged.
- If **no** subgraph provides a matching header, `default` is used (when provided).
- If `rename` is set, the header is returned under the new name.

**Never-join headers** (e.g. `set-cookie`) are never comma-joined:
multiple values are returned as separate header fields regardless of `algorithm`.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**propagate**](#option1propagate)|`object`|Propagate headers from subgraph responses to the final client response.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
propagate: {}

```


   
**Option 2 (alternative):** 
Remove headers before sending the response to the client.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**remove**](#option2remove)|`object`|Remove headers matched by the specification.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
remove: {}

```


   
**Option 3 (alternative):** 
Add or overwrite a header in the response to the client.

For never-join headers, appends another occurrence (multiple lines).


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**insert**](#option3insert)|`object`|Insert a header with a static value.<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
insert: {}

```


<a name="option1propagate"></a>
## Option 1: propagate: object

Propagate headers from subgraph responses to the final client response.

**Behavior**
- If multiple subgraphs return the header, values are merged using `algorithm`.
  Never-join headers are **never** comma-joined.
- If **no** subgraph returns a match, `default` (if set) is emitted.
- If `rename` is set, the outgoing header uses the new name.

### Examples
```yaml
# Forward Cache-Control from whichever subgraph supplies it (last wins)
propagate:
  named: Cache-Control
  algorithm: last

# Combine list-valued headers
propagate:
  named: vary
  algorithm: append

# Ensure a fallback header is always present
propagate:
  named: x-backend
  algorithm: append
  default: unknown
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**algorithm**||How to merge values across multiple subgraph responses.<br/>|yes|
|**default**|`string`, `null`|If no subgraph returns the header, set this default value.<br/>|no|
|[**exclude**](#option1propagateexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>|no|
|**matching**||Match headers by regex pattern(s) (OR).<br/>|no|
|**named**||Match headers by exact name (OR).<br/>|no|
|**rename**|`string`, `null`|Optionally rename the header when returning it to the client.<br/>|no|

<a name="option1propagateexclude"></a>
### Option 1: propagate\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option2remove"></a>
## Option 2: remove: object

Remove headers matched by the specification.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**exclude**](#option2removeexclude)|`string[]`|Exclude headers matching these regexes, applied after `matching`.<br/>||
|**matching**||Match headers by regex pattern(s) (OR).<br/>||
|**named**||Match headers by exact name (OR).<br/>||

<a name="option2removeexclude"></a>
### Option 2: remove\.exclude\[\]: array,null

Exclude headers matching these regexes, applied after `matching`.


**Items**

**Item Type:** `string`  
<a name="option3insert"></a>
## Option 3: insert: object

Insert a header with a static value.

### Examples
```yaml
- insert:
    name: x-env
    value: prod
```

```yaml
- insert:
    name: set-cookie
    value: "a=1; Path=/"
# If another Set-Cookie exists, this creates another header line (never joined)
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**algorithm**||How to merge values across multiple subgraph responses.<br/>Default: `Last` (overwrite).<br/>|no|
|**name**|`string`|Header name to insert or overwrite (case-insensitive).<br/>|yes|

   
**Option 1 (optional):** 
Static value provided in the config.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**value**|`string`||yes|


   
**Option 2 (optional):** 
A dynamic value computed by a VRL expression.

This allows you to generate header values based on the incoming request,
subgraph name, and (for response rules) subgraph response headers.
The expression has access to a context object with `.request`, `.subgraph`,
and `.response` fields.

For more information on the available functions and syntax, see the
[VRL documentation](https://vrl.dev/).

### Example
```yaml
# Insert a header with a value derived from another header.
- insert:
    name: x-auth-scheme
    expression: 'split(.request.headers.authorization, " ")[0] ?? "none"'
```


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**expression**|`string`|The VRL expression string.<br/>|yes|


<a name="http"></a>
## http: object

Configuration for the HTTP server/listener.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**host**|`string`|The host address to bind the HTTP server to.<br/><br/>Can also be set via the `HOST` environment variable.<br/>Default: `"0.0.0.0"`<br/>||
|**port**|`integer`|The port to bind the HTTP server to.<br/><br/>Can also be set via the `PORT` environment variable.<br/><br/>If you are running the router inside a Docker container, please ensure that the port is exposed correctly using `-p <host_port>:<container_port>` flag.<br/>Default: `4000`<br/>Format: `"uint16"`<br/>Minimum: `0`<br/>Maximum: `65535`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
host: 0.0.0.0
port: 4000

```

<a name="jwt"></a>
## jwt: object

Configuration for JWT authentication plugin.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**allowed\_algorithms**](#jwtallowed_algorithms)|`string[]`|List of allowed algorithms for verifying the JWT signature.<br/>Default: `"HS256"`, `"HS384"`, `"HS512"`, `"RS256"`, `"RS384"`, `"RS512"`, `"ES256"`, `"ES384"`, `"PS256"`, `"PS384"`, `"PS512"`, `"EdDSA"`<br/>|no|
|[**audiences**](#jwtaudiences)|`string[]`|The list of [JWT audiences](https://tools.ietf.org/html/rfc7519#section-4.1.3) are allowed to access.<br/>|no|
|**enabled**|`boolean`|Default: `false`<br/>|no|
|[**forward\_claims\_to\_upstream\_extensions**](#jwtforward_claims_to_upstream_extensions)|`object`|Forward the JWT claims to the upstream service using GraphQL's `.extensions`.<br/>Default: `{"enabled":false,"field_name":"jwt"}`<br/>|yes|
|[**issuers**](#jwtissuers)|`string[]`|Specify the [principal](https://tools.ietf.org/html/rfc7519#section-4.1.1) that issued the JWT, usually a URL or an email address.<br/>|no|
|[**jwks\_providers**](#jwtjwks_providers)|`array`|A list of JWKS providers to use for verifying the JWT signature.<br/>|yes|
|[**lookup\_locations**](#jwtlookup_locations)|`array`|A list of locations to look up for the JWT token in the incoming HTTP request.<br/>Default: `{"name":"authorization","prefix":"Bearer","source":"header"}`<br/>|no|
|**require\_authentication**|`boolean`, `null`|If set to `true`, the entire request will be rejected if the JWT token is not present in the request.<br/>|no|

**Additional Properties:** not allowed  
**Example**

```yaml
allowed_algorithms:
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
enabled: false
forward_claims_to_upstream_extensions:
  enabled: false
  field_name: jwt
lookup_locations:
  - name: authorization
    prefix: Bearer
    source: header

```

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
<a name="jwtforward_claims_to_upstream_extensions"></a>
### jwt\.forward\_claims\_to\_upstream\_extensions: object

Forward the JWT claims to the upstream service using GraphQL's `.extensions`.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**enabled**|`boolean`||yes|
|**field\_name**|`string`||yes|

**Example**

```yaml
enabled: false
field_name: jwt

```

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
|**name**|`string`|A valid HTTP header name, according to RFC 7230.<br/>Pattern: `^[A-Za-z0-9!#$%&'*+\-.^_\`\|~]+$`<br/>|yes|
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
- name: authorization
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
|**filter**|`string`, `null`|The filter to apply to log messages.<br/><br/>Can also be set via the `LOG_FILTER` environment variable.<br/>||
|**format**|`string`|The format of the log messages.<br/><br/>Can also be set via the `LOG_FORMAT` environment variable.<br/>Default: `"json"`<br/>Enum: `"pretty-tree"`, `"pretty-compact"`, `"json"`<br/>||
|**level**|`string`|The level of logging to use.<br/><br/>Can also be set via the `LOG_LEVEL` environment variable.<br/>Default: `"info"`<br/>Enum: `"trace"`, `"debug"`, `"info"`, `"warn"`, `"error"`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
filter: null
format: json
level: info

```

<a name="override_labels"></a>
## override\_labels: object

Configuration for overriding labels.


**Additional Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**Additional Properties**||Defines the value for a label override.<br/><br/>It can be a simple boolean,<br/>or an object containing the expression that evaluates to a boolean.<br/>||

<a name="override_subgraph_urls"></a>
## override\_subgraph\_urls: object

Configuration for overriding subgraph URLs.


**Additional Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**Additional Properties**](#override_subgraph_urlsadditionalproperties)|`object`||yes|

**Example**

```yaml
accounts:
  url: https://accounts.example.com/graphql
products:
  url:
    expression: |2-

              if .request.headers."x-region" == "us-east" {
                  "https://products-us-east.example.com/graphql"
              } else if .request.headers."x-region" == "eu-west" {
                  "https://products-eu-west.example.com/graphql"
              } else {
                .original_url
              }
          

```

<a name="override_subgraph_urlsadditionalproperties"></a>
### override\_subgraph\_urls\.additionalProperties: object

**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**url**||Overrides for the URL of the subgraph.<br/><br/>For convenience, a plain string in your configuration will be treated as a static URL.<br/><br/>### Static URL Example<br/>```yaml<br/>url: "https://api.example.com/graphql"<br/>```<br/><br/>### Dynamic Expression Example<br/><br/>The expression has access to the following variables:<br/>- `request`: The incoming HTTP request, including headers and other metadata.<br/>- `original_url`: The original URL of the subgraph (from supergraph sdl).<br/><br/>```yaml<br/>url:<br/>  expression: \|<br/>    if .request.headers."x-region" == "us-east" {<br/>      "https://products-us-east.example.com/graphql"<br/>    } else if .request.headers."x-region" == "eu-west" {<br/>      "https://products-eu-west.example.com/graphql"<br/>    } else {<br/>      .original_url<br/>    }<br/>|yes|

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
|**path**|`string`|The path to the supergraph file.<br/><br/>Can also be set using the `SUPERGRAPH_FILE_PATH` environment variable.<br/>Format: `"path"`<br/>|yes|
|[**poll\_interval**](#option1poll_interval)|`object`, `null`|Optional interval at which the file should be polled for changes.<br/>|yes|
|**source**|`string`|Constant Value: `"file"`<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
poll_interval: null

```


   
**Option 2 (alternative):** 
Loads a supergraph from Hive Console CDN.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**endpoint**|`string`|The CDN endpoint from Hive Console target.<br/><br/>Can also be set using the `HIVE_CDN_ENDPOINT` environment variable.<br/>|yes|
|**key**|`string`|The CDN Access Token with from the Hive Console target.<br/><br/>Can also be set using the `HIVE_CDN_KEY` environment variable.<br/>|yes|
|[**poll\_interval**](#option2poll_interval)|`object`|Interval at which the Hive Console should be polled for changes.<br/>Default: `"10s"`<br/>|yes|
|[**retry\_policy**](#option2retry_policy)|`object`|Interval at which the Hive Console should be polled for changes.<br/>Default: `{"max_retries":10}`<br/>|yes|
|**source**|`string`|Constant Value: `"hive"`<br/>|yes|
|[**timeout**](#option2timeout)|`object`|Request timeout for the Hive Console CDN requests.<br/>Default: `"1m"`<br/>|yes|

**Additional Properties:** not allowed  
**Example**

```yaml
poll_interval: 10s
retry_policy:
  max_retries: 10
timeout: 1m

```


<a name="option1poll_interval"></a>
## Option 1: poll\_interval: object,null

Optional interval at which the file should be polled for changes.
If not provided, the file will only be loaded once when the router starts.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**nanos**|`integer`|Format: `"uint32"`<br/>Minimum: `0`<br/>|yes|
|**secs**|`integer`|Format: `"uint64"`<br/>Minimum: `0`<br/>|yes|

**Example**

```yaml
{}

```

<a name="option2poll_interval"></a>
## Option 2: poll\_interval: object

Interval at which the Hive Console should be polled for changes.

Can also be set using the `HIVE_CDN_POLL_INTERVAL` environment variable.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**nanos**|`integer`|Format: `"uint32"`<br/>Minimum: `0`<br/>|yes|
|**secs**|`integer`|Format: `"uint64"`<br/>Minimum: `0`<br/>|yes|

**Example**

```yaml
10s

```

<a name="option2retry_policy"></a>
## Option 2: retry\_policy: object

Interval at which the Hive Console should be polled for changes.

By default, an exponential backoff retry policy is used, with 10 attempts.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**max\_retries**|`integer`|The maximum number of retries to attempt.<br/><br/>Retry mechanism is based on exponential backoff, see https://docs.rs/retry-policies/latest/retry_policies/policies/struct.ExponentialBackoff.html for additional details.<br/>Format: `"uint32"`<br/>Minimum: `0`<br/>|yes|

**Example**

```yaml
max_retries: 10

```

<a name="option2timeout"></a>
## Option 2: timeout: object

Request timeout for the Hive Console CDN requests.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**nanos**|`integer`|Format: `"uint32"`<br/>Minimum: `0`<br/>|yes|
|**secs**|`integer`|Format: `"uint64"`<br/>Minimum: `0`<br/>|yes|

**Example**

```yaml
1m

```

<a name="traffic_shaping"></a>
## traffic\_shaping: object

Configuration for the traffic-shaping of the executor. Use these configurations to control how requests are being executed to subgraphs.


**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|**dedupe\_enabled**|`boolean`|Enables/disables request deduplication to subgraphs.<br/><br/>When requests exactly matches the hashing mechanism (e.g., subgraph name, URL, headers, query, variables), and are executed at the same time, they will<br/>be deduplicated by sharing the response of other in-flight requests.<br/>Default: `true`<br/>||
|**max\_connections\_per\_host**|`integer`|Limits the concurrent amount of requests/connections per host/subgraph.<br/>Default: `100`<br/>Format: `"uint"`<br/>Minimum: `0`<br/>||
|**pool\_idle\_timeout\_seconds**|`integer`|Timeout for idle sockets being kept-alive.<br/>Default: `50`<br/>Format: `"uint64"`<br/>Minimum: `0`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
dedupe_enabled: true
max_connections_per_host: 100
pool_idle_timeout_seconds: 50

```


