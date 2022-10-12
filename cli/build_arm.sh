#!/bin/sh

set -e

IMG=ghcr.io/vercel/turbo-cross:v1.18.5-arm64
#IMG=ghcr.io/gsoltis/goreleaser-cross:v1.18.5-arm64

docker run \
  --rm \
  --privileged \
  -e CGO_ENABLED=1 \
  -e GORELEASER_KEY=${GORELEASER_KEY} \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v $(dirname `pwd`):/go/src/turborepo \
  -w /go/src/turborepo/cli \
  --platform linux/arm64 \
  --entrypoint /bin/bash \
  -it \
  $IMG


# docker run \
#   --rm \
#   --privileged \
#   -e CGO_ENABLED=1 \
#   -e GORELEASER_KEY=${GORELEASER_KEY} \
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   -v $(dirname `pwd`):/go/src/turborepo \
#   -w /go/src/turborepo/cli \
#   --platform linux/arm64 \
#   $IMG \
#   build --rm-dist -f arm-release.yml --snapshot --id turbo-arm64
# #  release  --rm-dist --snapshot -f arm-release.yml


# /usr/bin/docker
# run
# --name ghcriogsoltisturbocrossv1185_c4b810
# --label 786a9b
# --workdir /github/workspace
# --rm -e "PNPM_HOME"
# -e "NPM_CONFIG_USERCONFIG"
# -e "NODE_AUTH_TOKEN"
# -e "GORELEASER_KEY"
# -e "INPUT_ENTRYPOINT"
# -e "INPUT_ARGS"
# -e "HOME"
# -e "GITHUB_JOB"
# -e "GITHUB_REF"
# -e "GITHUB_SHA"
# -e "GITHUB_REPOSITORY"
# -e "GITHUB_REPOSITORY_OWNER"
# -e "GITHUB_RUN_ID"
# -e "GITHUB_RUN_NUMBER" -e "GITHUB_RETENTION_DAYS" -e "GITHUB_RUN_ATTEMPT"
# -e "GITHUB_ACTOR" -e "GITHUB_TRIGGERING_ACTOR" -e "GITHUB_WORKFLOW"
# -e "GITHUB_HEAD_REF" -e "GITHUB_BASE_REF" -e "GITHUB_EVENT_NAME" -e "GITHUB_SERVER_URL"
# -e "GITHUB_API_URL" -e "GITHUB_GRAPHQL_URL" -e "GITHUB_REF_NAME" -e "GITHUB_REF_PROTECTED"
# -e "GITHUB_REF_TYPE" -e "GITHUB_WORKSPACE" -e "GITHUB_ACTION" -e "GITHUB_EVENT_PATH"
# -e "GITHUB_ACTION_REPOSITORY" -e "GITHUB_ACTION_REF" -e "GITHUB_PATH" -e "GITHUB_ENV"
# -e "GITHUB_STEP_SUMMARY" -e "RUNNER_OS" -e "RUNNER_ARCH" -e "RUNNER_NAME" -e "RUNNER_TOOL_CACHE"
# -e "RUNNER_TEMP" -e "RUNNER_WORKSPACE" -e "ACTIONS_RUNTIME_URL" -e "ACTIONS_RUNTIME_TOKEN"
# -e "ACTIONS_CACHE_URL"
# -e GITHUB_ACTIONS=true -e CI=true
# --entrypoint "/bin/bash"
# -v "/var/run/docker.sock":"/var/run/docker.sock"
# -v "/home/runner/work/_temp/_github_home":"/github/home"
# -v "/home/runner/work/_temp/_github_workflow":"/github/workflow"
# -v "/home/runner/work/_temp/_runner_file_commands":"/github/file_commands"
# -v "/home/runner/work/turborepo/turborepo":"/github/workspace"
# ghcr.io/gsoltis/turbo-cross:v1.18.5 cd cli && make snapshot-turbo
