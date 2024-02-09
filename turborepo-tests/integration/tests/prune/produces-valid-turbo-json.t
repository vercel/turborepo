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

Modify turbo.json to add some fields to remoteCache and add a spaceId
  $ rm -rf out
  $ cat turbo.json | jq '.remoteCache.enabled = true | .remoteCache.timeout = 1000 | .remoteCache.apiUrl = "my-domain.com/cache" | .experimentalSpaces.id = "my-space-id"' > turbo.json.tmp
  $ mv turbo.json.tmp turbo.json
  $ ${TURBO} prune docs > /dev/null
  $ cat out/turbo.json | jq '.remoteCache | keys'
  [
    "apiUrl",
    "enabled",
    "timeout"
  ]
  $ cat out/turbo.json | jq '.remoteCache.enabled'
  true
  $ cat out/turbo.json | jq '.experimentalSpaces.id'
  "my-space-id"
  $ cat out/turbo.json | jq '.remoteCache.timeout'
  1000
  $ cat out/turbo.json | jq '.remoteCache.apiUrl'
  "my-domain.com/cache"

Modify turbo.json to add a remoteCache.enabled field set to false
  $ rm -rf out
  $ cat turbo.json | jq '.remoteCache.enabled = false' > turbo.json.tmp
  $ mv turbo.json.tmp turbo.json
  $ ${TURBO} prune docs > /dev/null
  $ cat out/turbo.json | jq '.remoteCache | keys'
  [
    "apiUrl",
    "enabled",
    "timeout"
  ]
  $ cat out/turbo.json | jq '.remoteCache.enabled'
  false
