Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Baseline task hashes
  $ cp "$TESTDIR/fixture-configs/a-baseline.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
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
=======
    "hash": "843536e46620dad2"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
  {
    "taskId": "my-app#build",
    "hash": "bbfabe4612171fc1"
  }
  {
    "taskId": "util#build",
<<<<<<< HEAD
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
=======
    "hash": "98d1cf4886bbc73d"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }

Change only my-app#build
  $ cp "$TESTDIR/fixture-configs/b-change-only-my-app.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
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
=======
    "hash": "843536e46620dad2"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
  {
    "taskId": "my-app#build",
    "hash": "0455e87c8abba36d"
  }
  {
    "taskId": "util#build",
<<<<<<< HEAD
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
=======
    "hash": "98d1cf4886bbc73d"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }

Change my-app#build dependsOn
  $ cp "$TESTDIR/fixture-configs/c-my-app-depends-on.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
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
=======
    "hash": "843536e46620dad2"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
  {
    "taskId": "my-app#build",
    "hash": "8d584a4d18836787"
  }
  {
    "taskId": "util#build",
<<<<<<< HEAD
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
=======
    "hash": "98d1cf4886bbc73d"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }

Non-materially modifying the dep graph does nothing.
  $ cp "$TESTDIR/fixture-configs/d-depends-on-util.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
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
=======
    "hash": "843536e46620dad2"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
  {
    "taskId": "my-app#build",
    "hash": "8d584a4d18836787"
  }
  {
    "taskId": "util#build",
<<<<<<< HEAD
    "hash": "e09943c27ed0a75d"
>>>>>>> 2eae5cbd82 (Update tests)
=======
    "hash": "98d1cf4886bbc73d"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }


Change util#build impacts itself and my-app
  $ cp "$TESTDIR/fixture-configs/e-depends-on-util-but-modified.json" "$(pwd)/turbo.json" && git commit -am "no comment" --quiet
  $ ${TURBO} build --dry=json | jq -r '.tasks | sort_by(.taskId)[] | {taskId, hash}'
  {
    "taskId": "another#build",
<<<<<<< HEAD
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
=======
    "hash": "843536e46620dad2"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
  {
    "taskId": "my-app#build",
    "hash": "239ff972999c4203"
  }
  {
    "taskId": "util#build",
<<<<<<< HEAD
    "hash": "b781fbdbf3ba6a42"
>>>>>>> 2eae5cbd82 (Update tests)
=======
    "hash": "70eb762a20d17252"
>>>>>>> 37c3c596f1 (chore: update integration tests)
  }
