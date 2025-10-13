# Support Non-Standard Requests with a Custom Plugin

This example demonstrates how to create a custom plugin that maps non-standard content types to standard ones so that they can be processed by the router. The plugin checks the `Content-Type` header of incoming HTTP requests, and if it matches a non-standard content type defined in the plugin's configuration, it replaces it with the desired ones. This allows clients to send GraphQL requests with custom content types while still being compatible with the router's processing capabilities.

## Configuration

The plugin is configured with a mapping of non-standard content types to standard ones. For example, you can map `text/plain` to `application/json`:

```yaml
plugins:
  non_standard_request:
    enabled: true
    config:
      # If the client sends non-standard content types
      # map them to the compatible ones
      content_type_map:
        "text/plain": "application/json"
```

So if a request comes in with `Content-Type: text/plain;charset=UTF-8`, the plugin will change it to `Content-Type: application/json;charset=UTF-8` before the router processes it.

## Implementation

The plugin implements the `RouterPlugin` trait and defines the `on_http_request` hook to perform the content type mapping. It checks the incoming request's `Content-Type` header, looks it up in the configured mapping, and if a match is found, it updates the header accordingly.