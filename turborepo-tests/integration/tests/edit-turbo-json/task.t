Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "54a66c2a0ea58cbf"
  }
  {
    "taskId": "my-app#build",
    "hash": "001416f982aed69c"
  }
  {
    "taskId": "util#build",
    "hash": "66888ae9d76026cd"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "54a66c2a0ea58cbf"
  }
  {
    "taskId": "my-app#build",
    "hash": "d2d2ec77471f3100"
  }
  {
    "taskId": "util#build",
    "hash": "66888ae9d76026cd"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "54a66c2a0ea58cbf"
  }
  {
    "taskId": "my-app#build",
    "hash": "cdb52550e7d85762"
  }
  {
    "taskId": "util#build",
    "hash": "66888ae9d76026cd"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "54a66c2a0ea58cbf"
  }
  {
    "taskId": "my-app#build",
    "hash": "cdb52550e7d85762"
  }
  {
    "taskId": "util#build",
    "hash": "66888ae9d76026cd"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "54a66c2a0ea58cbf"
  }
  {
    "taskId": "my-app#build",
    "hash": "313573ad0425e33f"
  }
  {
    "taskId": "util#build",
    "hash": "245c7c39bc82c32a"
  }
