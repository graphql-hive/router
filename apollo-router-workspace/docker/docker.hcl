variable "RELEASE" {
  default = "dev"
}

variable "PWD" {
  default = "."
}

variable "DOCKER_REGISTRY" {
  default = ""
}

variable "COMMIT_SHA" {
  default = ""
}

variable "BRANCH_NAME" {
  default = ""
}

variable "BUILD_TYPE" {
  # Can be "", "ci" or "publish"
  default = ""
}

variable "BUILD_STABLE" {
  # Can be "" or "1"
  default = ""
}

variable "IMAGE_SUFFIX" {
  default = ""
}

variable "BUILD_PLATFORM" {
  default = "linux/amd64,linux/arm64"
}

function "get_target" {
  params = []
  result = notequal("", BUILD_TYPE) ? notequal("ci", BUILD_TYPE) ? "target-publish" : "target-ci" : "target-dev"
}

function "get_platform" {
  params = []
  result = "${BUILD_PLATFORM}"
}

function "local_image_tag" {
  params = [name]
  result = equal("", BUILD_TYPE) ? "${DOCKER_REGISTRY}${name}:latest${IMAGE_SUFFIX}" : ""
}

function "stable_image_tag" {
  params = [name]
  result = equal("1", BUILD_STABLE) ? "${DOCKER_REGISTRY}${name}:latest${IMAGE_SUFFIX}" : ""
}

function "image_tag" {
  params = [name, tag]
  result = notequal("", tag) ? "${DOCKER_REGISTRY}${name}:${tag}${IMAGE_SUFFIX}" : ""
}

target "router-base" {
  dockerfile = "${PWD}/docker/router.dockerfile"
  args = {
    RELEASE = "${RELEASE}"
  }
}

target "target-dev" {}

target "target-ci" {
  cache-from = ["type=gha,ignore-error=true"]
  cache-to = ["type=gha,mode=max,ignore-error=true"]
}

target "target-publish" {
  platforms = [get_platform()]
  cache-from = ["type=gha,ignore-error=true"]
  cache-to = ["type=gha,mode=max,ignore-error=true"]
}

target "apollo-router" {
  inherits = ["router-base", get_target()]
  contexts = {
    router_pkg = "${PWD}/bin/router"
    config = "${PWD}"
    root_dir = "${PWD}/.."
  }
  args = {
    IMAGE_TITLE = "graphql-hive/apollo-router"
    PORT = "4000"
    IMAGE_DESCRIPTION = "Apollo Router for GraphQL Hive."
  }
  tags = [
    local_image_tag("apollo-router"),
    stable_image_tag("apollo-router"),
    image_tag("apollo-router", COMMIT_SHA),
    image_tag("apollo-router", BRANCH_NAME)
  ]
}

group "apollo-router-hive-build" {
  targets = [
    "apollo-router"
  ]
}
