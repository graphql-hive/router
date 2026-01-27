## Release

We are using the following tools to make releases:

- GitHub Releases
- GitHub Packages (for Docker runtime)
- Knope (for changesets and versioning)
- Cargo `publish` command
- Custom scripts

## Documenting a change

To document a change, start by running `knope changeset` and describe your changes. Make sure to pick patch/minor/major according to the [Semantic Versioning](https://semver.org/) specification.

Then, commit the changeset and push it to the Pull Request. The changeset will be automatically picked up by Knope. In case of a missing changeset in a PR, Knope will fail the release check. 

When a change is merged through a Pull Request, and a changeset file exists, Knope will create/update the release PR. 

> Knope does not manage the dependencies explicitly, but it will update the `Cargo.toml` file with the new version.  Check `.changeset/release-sync.sh` for additional details on how to automatically bump transient dependencies. 
 
## Release pipeline

Once the release PR is merged, the release process will be triggered (see `.github/workflows/release.yaml` and `.github/workflows/build-router.yaml`). The release pipeline is based on the following:

1. Knope release PR is merged into `main`
    1. Knope will create a GitHub release + tag
        1. The newly created release will run `.github/workflows/build-router.yaml` workflow
        2. The pipeline will build the router binary and upload it to the release assets
        3. The pipeline will build a new Docker image for the router and push it to the GitHub Packages
    2. The `.github/workflows/release.yaml` pipeline runs to release Crates
        1. It will look for unreleased packages in the Crates
        2. It will publish new Crates to Crates.io in the right order
