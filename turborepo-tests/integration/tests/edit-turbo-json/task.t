Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "90d25abdd579d2bf"
  }
  {
    "taskId": "my-app#build",
    "hash": "4dc68e628703cbf4"
  }
  {
    "taskId": "util#build",
    "hash": "728076a89c49afbf"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "90d25abdd579d2bf"
  }
  {
    "taskId": "my-app#build",
    "hash": "467895752c6e4d42"
  }
  {
    "taskId": "util#build",
    "hash": "728076a89c49afbf"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "90d25abdd579d2bf"
  }
  {
    "taskId": "my-app#build",
    "hash": "9ff4347eebe225a1"
  }
  {
    "taskId": "util#build",
    "hash": "728076a89c49afbf"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "90d25abdd579d2bf"
  }
  {
    "taskId": "my-app#build",
    "hash": "9ff4347eebe225a1"
  }
  {
    "taskId": "util#build",
    "hash": "728076a89c49afbf"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "90d25abdd579d2bf"
  }
  {
    "taskId": "my-app#build",
    "hash": "fa7a059de209da9b"
  }
  {
    "taskId": "util#build",
    "hash": "7b57a7ca5e311e4d"
  }
