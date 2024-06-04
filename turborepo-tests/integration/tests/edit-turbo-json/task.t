Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "3639431fdcdf9f9e"
  }
  {
    "taskId": "my-app#build",
    "hash": "0555ce94ca234049"
  }
  {
    "taskId": "util#build",
    "hash": "bf1798d3e46e1b48"
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "3639431fdcdf9f9e"
  }
  {
    "taskId": "my-app#build",
    "hash": "6eea03fab6f9a8c8"
  }
  {
    "taskId": "util#build",
    "hash": "bf1798d3e46e1b48"
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "3639431fdcdf9f9e"
  }
  {
    "taskId": "my-app#build",
    "hash": "8637a0f5db686164"
  }
  {
    "taskId": "util#build",
    "hash": "bf1798d3e46e1b48"
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "3639431fdcdf9f9e"
  }
  {
    "taskId": "my-app#build",
    "hash": "8637a0f5db686164"
  }
  {
    "taskId": "util#build",
    "hash": "bf1798d3e46e1b48"
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
    "hash": "3639431fdcdf9f9e"
  }
  {
    "taskId": "my-app#build",
    "hash": "2721f01b53b758d0"
  }
  {
    "taskId": "util#build",
    "hash": "74c8eb9bab702b4b"
  }
