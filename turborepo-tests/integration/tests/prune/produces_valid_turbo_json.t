Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) monorepo_with_root_dep

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune docs
  Generating pruned monorepo for docs in .*/out (re)
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
