# HiveRouterConfig

**Properties**

|Name|Type|Description|Required|
|----|----|-----------|--------|
|[**headers**](#headers)|`object`|Configuration for the headers.<br/>Default: `{}`<br/>||
|[**http**](#http)|`object`|Configuration for the HTTP server/listener.<br/>Default: `{"host":"0.0.0.0","port":4000}`<br/>||
|[**jwt**](#jwt)|`object`, `null`|Configuration for JWT authentication plugin.<br/>|yes|
|[**log**](#log)|`object`|The router logger configuration.<br/>Default: `{"filter":null,"format":"json","level":"info"}`<br/>||
|[**query\_planner**](#query_planner)|`object`|Query planning configuration.<br/>Default: `{"allow_expose":false,"timeout":"10s"}`<br/>||
|[**supergraph**](#supergraph)|`object`|Configuration for the Federation supergraph source. By default, the router will use a local file-based supergraph source (`./supergraph.graphql`).<br/>Default: `{"path":"supergraph.graphql","source":"file"}`<br/>||
|[**traffic\_shaping**](#traffic_shaping)|`object`|Configuration for the traffic-shaper executor. Use these configurations to control how requests are being executed to subgraphs.<br/>Default: `{"dedupe_enabled":true,"max_connections_per_host":100,"pool_idle_timeout_seconds":50}`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
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
  max_connections_per_host: 100
  pool_idle_timeout_seconds: 50

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
|**expression**|`string`||yes|


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
|**expression**|`string`||yes|


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
|**expression**|`string`||yes|


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
|**expression**|`string`||yes|


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
|**max\_connections\_per\_host**|`integer`|Limits the concurrent amount of requests/connections per host/subgraph.<br/>Default: `100`<br/>Format: `"uint"`<br/>Minimum: `0`<br/>||
|**pool\_idle\_timeout\_seconds**|`integer`|Timeout for idle sockets being kept-alive.<br/>Default: `50`<br/>Format: `"uint64"`<br/>Minimum: `0`<br/>||

**Additional Properties:** not allowed  
**Example**

```yaml
dedupe_enabled: true
max_connections_per_host: 100
pool_idle_timeout_seconds: 50

```


