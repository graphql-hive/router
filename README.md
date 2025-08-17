![Hive GraphQL Platform](https://the-guild.dev/graphql/hive/github-org-image.png)

# Hive Gateway (Rust)

A fully open-source MIT-licensed GraphQL API gateway that can act as a [GraphQL federation](https://the-guild.dev/graphql/hive/federation) Gateway, built with Rust for maximum performance and robustness.

It can be run as a standalone binary or a Docker Image. Query planner can be used as a standalone Crate library.

## Try it out

### Binary

See [GitHub Releases](https://github.com/graphql-hive/gateway-rs/releases) and use the artifacts published to each release.

### Docker

The gateway is published via [Docker to GitHub Container Registry](). You may use it directly using the following command:

```bash
docker run -p 4000:4000 -v ./my-supergraph.graphql:/app/config/supergraph.graphql ghcr.io/graphql-hive/router:latest
```

Replace `my-supergraph.graphql` with a local supergraph file.

Replace `latest` with a specific version tag, or a pre-release for one of the PRs (`pr-<number>` or `sha-<commit-sha>`).

> To try the query planner, see [bin/dev-cli/README.md](bin/dev-cli/README.md) for instructions to quickly use the qp-dev-cli for seeing the QP in action.

## Local Development

* Run `cargo test --all` to execute all tests.
* Run `cargo test_qp` to execute all tests in the query planner.
* See [query-planner/src/tests/README.md](query-planner/src/tests/README.md) for more information, logging and configuration.
