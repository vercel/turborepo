Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "0220ac0b414d5b1e"
  }
  {
    "taskId": "my-app#build",
    "hash": "5337ec1c89125f9b"
  }
  {
    "taskId": "util#build",
    "hash": "7d67ce9d6cdc1638"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "0220ac0b414d5b1e"
  }
  {
    "taskId": "my-app#build",
    "hash": "0212103ea7df2237"
  }
  {
    "taskId": "util#build",
    "hash": "7d67ce9d6cdc1638"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "0220ac0b414d5b1e"
  }
  {
    "taskId": "my-app#build",
    "hash": "04d324a5f9f448ed"
  }
  {
    "taskId": "util#build",
    "hash": "7d67ce9d6cdc1638"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "0220ac0b414d5b1e"
  }
  {
    "taskId": "my-app#build",
    "hash": "04d324a5f9f448ed"
  }
  {
    "taskId": "util#build",
    "hash": "7d67ce9d6cdc1638"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "0220ac0b414d5b1e"
  }
  {
    "taskId": "my-app#build",
    "hash": "ddef688b34f0a747"
  }
  {
    "taskId": "util#build",
    "hash": "9e00e57d9edd2c3a"
  }
