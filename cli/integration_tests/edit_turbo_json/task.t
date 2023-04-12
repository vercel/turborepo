Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "4c4f0e076ebc3a2a"
  }
  {
    "taskId": "my-app#build",
    "hash": "9431e2a02286769a"
  }
  {
    "taskId": "util#build",
    "hash": "e6ceb7aa9a6948f8"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "4c4f0e076ebc3a2a"
  }
  {
    "taskId": "my-app#build",
    "hash": "WAT"
  }
  {
    "taskId": "util#build",
    "hash": "e6ceb7aa9a6948f8"
  }
