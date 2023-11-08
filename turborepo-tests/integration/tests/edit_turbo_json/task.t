Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "02f55362198a6c3d"
  }
  {
    "taskId": "my-app#build",
    "hash": "90ff09567a6b2356"
  }
  {
    "taskId": "util#build",
    "hash": "9b9969f14caa05a4"
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
    "hash": "6c0ac038b6e27281"
  }
  {
    "taskId": "util#build",
    "hash": "9b9969f14caa05a4"
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
    "hash": "bcaf2a39bbcbcb58"
  }
  {
    "taskId": "util#build",
    "hash": "9b9969f14caa05a4"
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
    "hash": "bcaf2a39bbcbcb58"
  }
  {
    "taskId": "util#build",
    "hash": "9b9969f14caa05a4"
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
    "hash": "807226804bff5475"
  }
  {
    "taskId": "util#build",
    "hash": "0e5f606c75e19ed2"
  }
