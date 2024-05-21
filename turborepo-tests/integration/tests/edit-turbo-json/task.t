Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "61394a550211cbe8"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "1d7be3c12072f23c"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "ccf2441853eb8930"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "ccf2441853eb8930"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "71c6f7392eeebdc1"
  }
  {
    "taskId": "util#build",
    "hash": "73e9903a46832238"
  }
