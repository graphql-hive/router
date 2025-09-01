![Hive GraphQL Platform](https://the-guild.dev/graphql/hive/github-org-image.png)

# Hive Router (Rust)

A fully open-source MIT-licensed GraphQL API router that can act as a [GraphQL federation](https://the-guild.dev/graphql/hive/federation) Router, built with Rust for maximum performance and robustness.

> [!TIP]
> Interested in the benchmark results? Check out the [Federation Gateway Performance comparison](https://the-guild.dev/graphql/hive/federation-gateway-performance)

It can be run as a standalone binary or a Docker Image. Query planner can be used as a standalone Crate library.

[Binary Releases](https://github.com/graphql-hive/router/releases) | [Docker Releases](https://github.com/graphql-hive/router/pkgs/container/router) | [Configuration reference](./docs/README.md)

## Try it out

Download Hive Router using the following install script:

```
curl -o- https://raw.githubusercontent.com/graphql-hive/router/main/install.sh | sh
```

> At the moment, only Linux runtimes are supported using a binary, see Docker option below if you are using a different OS.

Create a simple configuration file that points to your supergraph schema file:

```yaml
# hive-router.config.yaml
supergraph:
  source: file
  path: ./supergraph.graphql
```

Alternativly, you can use environment variables to configure the router:

```env
HIVE_SUPERGRAPH_SOURCE=file
HIVE_SUPERGRAPH_PATH=./supergraph.graphql
```

Then, run the router:

```bash
# By default, "hive-router.config.yaml" is used for configuration. Override it by setting "HIVE_CONFIG_FILE_PATH=some-custom-file.yaml"
# If you are using env vars, make sure to set the variables before running the router.
./hive_router
```

### Binary

See [GitHub Releases](https://github.com/graphql-hive/router/releases) to the full list of release and versions.

### Docker

The router image is being published to [Docker to GitHub Container Registry](). You may use it directly using the following command:

```bash
docker run \
  -p 4000:4000 \
  -e HIVE_SUPERGRAPH_SOURCE="file" \
  -e HIVE_SUPERGRAPH_PATH="/app/supergraph.graphql" \
  -v ./my-supergraph.graphql:/app/supergraph.graphql \
  ghcr.io/graphql-hive/router:latest
```

> Replace `my-supergraph.graphql` with a local supergraph file.

Alternativly, you can mount the configuration file using `-v` and pass all other configurations there:

```bash
docker run \
  -p 4000:4000 \
  -v ./hive-router.config.yaml:/app/hive-router.config.yaml \
  ghcr.io/graphql-hive/router:latest
```

> Replace `latest` with a specific version tag, or a pre-release for one of the PRs (`pr-<number>` or `sha-<commit-sha>`).

> To try the query planner, see [bin/dev-cli/README.md](bin/dev-cli/README.md) for instructions to quickly use the qp-dev-cli for seeing the QP in action.

## Local Development

* Run `cargo test --all` to execute all tests.
* Run `cargo test_qp` to execute all tests in the query planner.
* See [query-planner/src/tests/README.md](query-planner/src/tests/README.md) for more information, logging and configuration.
