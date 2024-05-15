
Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh task_dependencies/overwriting

Test
  $ ${TURBO} run build > tmp.log
  $ cat tmp.log | grep "Packages in scope" -A2
  \xe2\x80\xa2 Packages in scope: workspace-a, workspace-b (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)

# workspace-a#generate ran
  $ cat tmp.log | grep "workspace-a:generate"
<<<<<<< HEAD
  workspace-a:generate: cache miss, executing 8e1618d20f6303dc
=======
  workspace-a:generate: cache miss, executing fcb7e12d96f4af12
>>>>>>> 2eae5cbd82 (Update tests)
  workspace-a:generate: 
  workspace-a:generate: > generate
  workspace-a:generate: > echo generate-workspace-a
  workspace-a:generate: 
  workspace-a:generate: generate-workspace-a
workspace-a#build ran
  $ cat tmp.log | grep "workspace-a:build"
<<<<<<< HEAD
  workspace-a:build: cache miss, executing 50df012517e672e6
=======
  workspace-a:build: cache miss, executing 5b44d28ebc0672bb
>>>>>>> 2eae5cbd82 (Update tests)
  workspace-a:build: 
  workspace-a:build: > build
  workspace-a:build: > echo build-workspace-a
  workspace-a:build: 
  workspace-a:build: build-workspace-a

workspace-b#generate DID NOT run
  $ cat tmp.log | grep "workspace-b:generate"
  [1]

workspace-b#build ran
  $ cat tmp.log | grep "workspace-b:build"
<<<<<<< HEAD
  workspace-b:build: cache miss, executing a4ecaf3902039f0c
=======
  workspace-b:build: cache miss, executing e4c450285346e321
>>>>>>> 2eae5cbd82 (Update tests)
  workspace-b:build: 
  workspace-b:build: > build
  workspace-b:build: > echo build-workspace-b
  workspace-b:build: 
  workspace-b:build: build-workspace-b
