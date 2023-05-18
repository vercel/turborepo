Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "6b41bfbba9683909"
  }
  {
    "taskId": "my-app#build",
    "hash": "122bd9fc20f4511c"
  }
  {
    "taskId": "util#build",
    "hash": "82bc93ff27e552d6"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "6b41bfbba9683909"
  }
  {
    "taskId": "my-app#build",
    "hash": "c5fde4c17183b12e"
  }
  {
    "taskId": "util#build",
    "hash": "82bc93ff27e552d6"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "6b41bfbba9683909"
  }
  {
    "taskId": "my-app#build",
    "hash": "8f05b56dd2249965"
  }
  {
    "taskId": "util#build",
    "hash": "82bc93ff27e552d6"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "6b41bfbba9683909"
  }
  {
    "taskId": "my-app#build",
    "hash": "8f05b56dd2249965"
  }
  {
    "taskId": "util#build",
    "hash": "82bc93ff27e552d6"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "6b41bfbba9683909"
  }
  {
    "taskId": "my-app#build",
    "hash": "3ca79679a5de5b02"
  }
  {
    "taskId": "util#build",
    "hash": "0aba93b4cdf3eac1"
  }
