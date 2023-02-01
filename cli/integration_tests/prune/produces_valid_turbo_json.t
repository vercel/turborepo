Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=docs
  Generating pruned monorepo for docs in .*/out (re)
   - Added docs
   - Added shared
   - Added util

Make sure we prune tasks that reference a pruned workspace
  $ cat out/turbo.json | jq
  {
    "pipeline": {
      "build": {
        "outputs": [],
        "cache": true,
        "dependsOn": [],
        "inputs": [],
        "outputMode": "full",
        "env": [],
        "persistent": false
      }
    },
    "remoteCache": {}
  }

Verify turbo can read the produced turbo.json
  $ cd out
  $ ${TURBO} build --dry=json | jq '.packages'
   WARNING  cannot find a .git folder. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo
  [
    "docs",
    "shared",
    "util"
  ]
