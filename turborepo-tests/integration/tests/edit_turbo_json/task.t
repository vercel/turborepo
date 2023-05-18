Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "bdbf7133a23da868"
  }
  {
    "taskId": "my-app#build",
    "hash": "c0aa511cf2721438"
  }
  {
    "taskId": "util#build",
    "hash": "ac6ceb0714bda4f3"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "bdbf7133a23da868"
  }
  {
    "taskId": "my-app#build",
    "hash": "56fb0a7f729b8ad6"
  }
  {
    "taskId": "util#build",
    "hash": "ac6ceb0714bda4f3"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "bdbf7133a23da868"
  }
  {
    "taskId": "my-app#build",
    "hash": "a52eac95b03a5d15"
  }
  {
    "taskId": "util#build",
    "hash": "ac6ceb0714bda4f3"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "bdbf7133a23da868"
  }
  {
    "taskId": "my-app#build",
    "hash": "a52eac95b03a5d15"
  }
  {
    "taskId": "util#build",
    "hash": "ac6ceb0714bda4f3"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "bdbf7133a23da868"
  }
  {
    "taskId": "my-app#build",
    "hash": "b87616c5e3bd8e8d"
  }
  {
    "taskId": "util#build",
    "hash": "85337dfdca9a7a98"
  }
