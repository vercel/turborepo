Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune docs
  Generating pruned monorepo for docs in .*(\/|\\)out (re)
   - Added docs
   - Added shared
   - Added util

Make sure we prune tasks that reference a pruned workspace
  $ cat out/turbo.json | jq
  {
    "$schema": "https://turbo.build/schema.json",
    "pipeline": {
      "build": {
        "outputs": []
      }
    }
  }

Verify turbo can read the produced turbo.json
  $ cd out
  $ ${TURBO} build --dry=json | jq '.packages'
  [
    "docs",
    "shared",
    "util"
  ]

Modify turbo.json to add a remoteCache.enabled field set to true
  $ rm -rf out
  $ cat turbo.json | jq '.remoteCache.enabled = true' > turbo.json.tmp
  $ mv turbo.json.tmp turbo.json
  $ ${TURBO} prune docs > /dev/null
  $ cat out/turbo.json | jq '.remoteCache | keys'
  [
    "apiUrl",
    "enabled",
    "loginUrl",
    "preflight",
    "signature",
    "teamId",
    "teamSlug",
    "timeout",
    "token"
  ]
  $ cat out/turbo.json | jq '.remoteCache.enabled'
  true

Modify turbo.json to add a remoteCache.enabled field set to false
  $ rm -rf out
  $ cat turbo.json | jq '.remoteCache.enabled = false' > turbo.json.tmp
  $ mv turbo.json.tmp turbo.json
  $ ${TURBO} prune docs > /dev/null
  $ cat out/turbo.json | jq '.remoteCache | keys'
  [
    "apiUrl",
    "enabled",
    "loginUrl",
    "preflight",
    "signature",
    "teamId",
    "teamSlug",
    "timeout",
    "token"
  ]
  $ cat out/turbo.json | jq '.remoteCache.enabled'
  false
