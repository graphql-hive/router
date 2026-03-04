# Subgraph-level JWT handling with Cookies Example

This example plugin extracts JWT information from cookies and adds it to the request context, making it available for subgraph-level authentication and authorization.

- We use `on_graphql_params` instead of `on_http_request` to make sure we handle GraphQL requests
- Then in this hook, we parse the cookies from the request headers, extract the JWT token, and add it to the plugin context
- If the given expiry time of the token is in the past, we call the refresh endpoint to get a new token, and add the new token to the plugin context
- Then for each subgraph execution, we add the JWT token to the outgoing request headers in `on_subgraph_execute` hook, we don't use `on_subgraph_http_request` because `on_subgraph_http_request` is too late to determine the headers of the request for the deduplication. So if we change the header here, it won't be considered for the deduplication, and we might end up with a single request with an unexpected token from the cache instead of two requests with the correct tokens.
- In the end, `on_http_request`'s `on_end`, we set the new token in the cookies of the response if we got a new token from the refresh endpoint.

In the cookies we keep three pieces of information:
- `jwt_token`: the JWT token itself, which is used for authentication and authorization in the subgraphs
- `jwt_expires_at`: the expiry time of the token, which is used to determine if we need to refresh the token
- `jwt_refresh_token`: the refresh token, which is used to get a new JWT token when the current token is expired.