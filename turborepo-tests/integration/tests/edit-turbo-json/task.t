Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "8a8944ef32696847"
  }
  {
    "taskId": "util#build",
    "hash": "1ce33e04f265f95c"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "83bb5352c916557e"
  }
  {
    "taskId": "util#build",
    "hash": "1ce33e04f265f95c"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "346838a5f9d9a530"
  }
  {
    "taskId": "util#build",
    "hash": "1ce33e04f265f95c"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "346838a5f9d9a530"
  }
  {
    "taskId": "util#build",
    "hash": "1ce33e04f265f95c"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "b15e1a917912cd09"
  }
  {
    "taskId": "util#build",
    "hash": "2ee29eb57d7f69b3"
  }
