Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "ea00e25531db048f"
  }
  {
    "taskId": "my-app#build",
    "hash": "270f1ef47a80f1d1"
  }
  {
    "taskId": "util#build",
    "hash": "fad2a643cb480b55"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "ea00e25531db048f"
  }
  {
    "taskId": "my-app#build",
    "hash": "b0eb2c24b2a84be5"
  }
  {
    "taskId": "util#build",
    "hash": "fad2a643cb480b55"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "ea00e25531db048f"
  }
  {
    "taskId": "my-app#build",
    "hash": "9e63702de36d25c6"
  }
  {
    "taskId": "util#build",
    "hash": "fad2a643cb480b55"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "ea00e25531db048f"
  }
  {
    "taskId": "my-app#build",
    "hash": "9e63702de36d25c6"
  }
  {
    "taskId": "util#build",
    "hash": "fad2a643cb480b55"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "ea00e25531db048f"
  }
  {
    "taskId": "my-app#build",
    "hash": "867bee2191fbd90c"
  }
  {
    "taskId": "util#build",
    "hash": "6f4abe279ba198a8"
  }
