Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "930c372b34f6fc8c"
  }
  {
    "taskId": "my-app#build",
    "hash": "1ec581da8a4765ab"
  }
  {
    "taskId": "util#build",
    "hash": "76ab904c7ecb2d51"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "930c372b34f6fc8c"
  }
  {
    "taskId": "my-app#build",
    "hash": "7beb921347d78bfa"
  }
  {
    "taskId": "util#build",
    "hash": "76ab904c7ecb2d51"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "930c372b34f6fc8c"
  }
  {
    "taskId": "my-app#build",
    "hash": "685b9438e3900bed"
  }
  {
    "taskId": "util#build",
    "hash": "76ab904c7ecb2d51"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "930c372b34f6fc8c"
  }
  {
    "taskId": "my-app#build",
    "hash": "685b9438e3900bed"
  }
  {
    "taskId": "util#build",
    "hash": "76ab904c7ecb2d51"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "930c372b34f6fc8c"
  }
  {
    "taskId": "my-app#build",
    "hash": "bd4e46c0e2fdd071"
  }
  {
    "taskId": "util#build",
    "hash": "f6cfba79415007a3"
  }
