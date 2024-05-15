Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "61394a550211cbe8"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
=======
    "hash": "39902e236d45b17a"
  }
  {
    "taskId": "my-app#build",
    "hash": "8f74c4a19d54432c"
  }
  {
    "taskId": "util#build",
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "1d7be3c12072f23c"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
=======
    "hash": "39902e236d45b17a"
  }
  {
    "taskId": "my-app#build",
    "hash": "c741bcbd31d15d60"
  }
  {
    "taskId": "util#build",
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "ccf2441853eb8930"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
=======
    "hash": "39902e236d45b17a"
  }
  {
    "taskId": "my-app#build",
    "hash": "6553128bdbc1036d"
  }
  {
    "taskId": "util#build",
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "ccf2441853eb8930"
  }
  {
    "taskId": "util#build",
    "hash": "d30fc4474534c30e"
=======
    "hash": "39902e236d45b17a"
  }
  {
    "taskId": "my-app#build",
    "hash": "6553128bdbc1036d"
  }
  {
    "taskId": "util#build",
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
    "hash": "1d62465edaa86a4e"
  }
  {
    "taskId": "my-app#build",
    "hash": "71c6f7392eeebdc1"
  }
  {
    "taskId": "util#build",
    "hash": "73e9903a46832238"
=======
    "hash": "39902e236d45b17a"
  }
  {
    "taskId": "my-app#build",
    "hash": "fa0fd889b37b9c1b"
  }
  {
    "taskId": "util#build",
    "hash": "b781fbdbf3ba6a42"
>>>>>>> 2eae5cbd82 (Update tests)
  }
