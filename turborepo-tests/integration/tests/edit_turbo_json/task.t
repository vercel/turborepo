Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "40e9f8cd871f3409"
  }
  {
    "taskId": "my-app#build",
    "hash": "c16786bd76a16cbc"
  }
  {
    "taskId": "util#build",
    "hash": "6f0f87b7790cbede"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "40e9f8cd871f3409"
  }
  {
    "taskId": "my-app#build",
    "hash": "7f244b79035d958a"
  }
  {
    "taskId": "util#build",
    "hash": "6f0f87b7790cbede"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "40e9f8cd871f3409"
  }
  {
    "taskId": "my-app#build",
    "hash": "cd876550342824e4"
  }
  {
    "taskId": "util#build",
    "hash": "6f0f87b7790cbede"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "40e9f8cd871f3409"
  }
  {
    "taskId": "my-app#build",
    "hash": "cd876550342824e4"
  }
  {
    "taskId": "util#build",
    "hash": "6f0f87b7790cbede"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "40e9f8cd871f3409"
  }
  {
    "taskId": "my-app#build",
    "hash": "db822eb022b5b1bc"
  }
  {
    "taskId": "util#build",
    "hash": "7a24dcdd6ca3c7aa"
  }
