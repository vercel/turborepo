Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "8d90a05aa60bd604"
  }
  {
    "taskId": "my-app#build",
    "hash": "bcfea334449257fe"
  }
  {
    "taskId": "util#build",
    "hash": "e64dab76e045fbb4"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "8d90a05aa60bd604"
  }
  {
    "taskId": "my-app#build",
    "hash": "943c0f8cc9e90a1e"
  }
  {
    "taskId": "util#build",
    "hash": "e64dab76e045fbb4"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "8d90a05aa60bd604"
  }
  {
    "taskId": "my-app#build",
    "hash": "61dafe4314e0156a"
  }
  {
    "taskId": "util#build",
    "hash": "e64dab76e045fbb4"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "8d90a05aa60bd604"
  }
  {
    "taskId": "my-app#build",
    "hash": "61dafe4314e0156a"
  }
  {
    "taskId": "util#build",
    "hash": "e64dab76e045fbb4"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "8d90a05aa60bd604"
  }
  {
    "taskId": "my-app#build",
    "hash": "f976edb1e8ddf783"
  }
  {
    "taskId": "util#build",
    "hash": "1f378ad2e5831e1f"
  }
