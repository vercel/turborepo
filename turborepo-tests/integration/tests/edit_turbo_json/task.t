Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "9b840a9eeed635db"
  }
  {
    "taskId": "my-app#build",
    "hash": "4ffefafd578043d5"
  }
  {
    "taskId": "util#build",
    "hash": "12af4a2f5c5af4e1"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "9b840a9eeed635db"
  }
  {
    "taskId": "my-app#build",
    "hash": "b412269dc6ab1fb0"
  }
  {
    "taskId": "util#build",
    "hash": "12af4a2f5c5af4e1"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "9b840a9eeed635db"
  }
  {
    "taskId": "my-app#build",
    "hash": "523d9c8f471c12dd"
  }
  {
    "taskId": "util#build",
    "hash": "12af4a2f5c5af4e1"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "9b840a9eeed635db"
  }
  {
    "taskId": "my-app#build",
    "hash": "523d9c8f471c12dd"
  }
  {
    "taskId": "util#build",
    "hash": "12af4a2f5c5af4e1"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "9b840a9eeed635db"
  }
  {
    "taskId": "my-app#build",
    "hash": "d053492baf0266c1"
  }
  {
    "taskId": "util#build",
    "hash": "1e36d9d1795aa2fd"
  }
