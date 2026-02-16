# Error Mapping Plugin Example

This example demonstrates how to implement a plugin that maps GraphQL errors to custom HTTP responses.
So you can change the status code, and the error code and message in the response body based on the error type or other criteria.
It uses `on_error` hook to intercept errors and modify the response accordingly.

```yaml
plugins:
  error_mapping:
    enabled: true
    config:
      GRAPHQL_PARSE_FAILED:
        status_code: 400
        code: InvalidInput
      DOWNSTREAM_SERVICE_ERROR:
        status_code: 502
        code: BadGateway
```

In this example;

```diff
{
    "errors": [
        {
            "message": "Failed to parse GraphQL query",
            "extensions": {
-                "code": "GRAPHQL_PARSE_FAILED",
+                "code": "InvalidInput",
            }
        }
    ]
}
```

And 

```diff
{
    "errors": [
        {
            "message": "<downstream service error message>",
            "extensions": {
-                "code": "DOWNSTREAM_SERVICE_ERROR",
+                "code": "BadGateway",
            }
        }
    ]
}
```