
Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) task_dependencies/overwriting

Test
  $ ${TURBO} run build > tmp.log
  $ cat tmp.log | grep "Packages in scope" -A2
  \xe2\x80\xa2 Packages in scope: workspace-a, workspace-b (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)

# workspace-a#generate ran
  $ cat tmp.log | grep "workspace-a:generate"
  workspace-a:generate: cache miss, executing 33cb31b9a1feefdc
  workspace-a:generate: 
  workspace-a:generate: > generate
  workspace-a:generate: > echo 'generate workspace-a'
  workspace-a:generate: 
  workspace-a:generate: generate workspace-a
workspace-a#build ran
  $ cat tmp.log | grep "workspace-a:build"
  workspace-a:build: cache miss, executing 6ef94e61ac123f30
  workspace-a:build: 
  workspace-a:build: > build
  workspace-a:build: > echo 'build workspace-a'
  workspace-a:build: 
  workspace-a:build: build workspace-a

workspace-b#generate DID NOT run
  $ cat tmp.log | grep "workspace-b:generate"
  [1]

workspace-b#build ran
  $ cat tmp.log | grep "workspace-b:build"
  workspace-b:build: cache miss, executing 000276a016e6bd0c
  workspace-b:build: 
  workspace-b:build: > build
  workspace-b:build: > echo 'build workspace-b'
  workspace-b:build: 
  workspace-b:build: build workspace-b
