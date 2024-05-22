Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "78b732d9478d9b83"
  }
  {
    "taskId": "my-app#build",
    "hash": "ed450f573b231cb7"
  }
  {
    "taskId": "util#build",
    "hash": "41b033e352a43533"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "78b732d9478d9b83"
  }
  {
    "taskId": "my-app#build",
    "hash": "eb391860afd5dfdc"
  }
  {
    "taskId": "util#build",
    "hash": "41b033e352a43533"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "78b732d9478d9b83"
  }
  {
    "taskId": "my-app#build",
    "hash": "d71bf2777e3824b7"
  }
  {
    "taskId": "util#build",
    "hash": "41b033e352a43533"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "78b732d9478d9b83"
  }
  {
    "taskId": "my-app#build",
    "hash": "d71bf2777e3824b7"
  }
  {
    "taskId": "util#build",
    "hash": "41b033e352a43533"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "78b732d9478d9b83"
  }
  {
    "taskId": "my-app#build",
    "hash": "550479ca3246010d"
  }
  {
    "taskId": "util#build",
    "hash": "d29ee2ca954217ef"
  }
