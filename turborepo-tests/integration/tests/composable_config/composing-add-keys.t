Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) composable_config

# The add-keys-task in the root turbo.json has no config. This test:
# [x] Tests dependsOn works by asserting that another task runs first
# [x] Tests outputs works by asserting that the right directory is cached
# [x] Tests outputMode by asserting output logs on a second run
# [x] Tests inputs works by changing a file and testing there was a cache miss
# [x] Tests env works by setting an env var and asserting there was a cache miss

# 1. First run, assert for `dependsOn` and `outputs` keys
  $ ${TURBO} run add-keys-task --filter=add-keys > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache miss, executing 78349dfba4d58116
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing c3b35d8aaef32d5b
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo "running add-keys-task" > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ HASH=$(cat tmp.log | grep -E "add-keys:add-keys-task.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/add-keys/.turbo/turbo-add-keys-task.log
  apps/add-keys/out/
  apps/add-keys/out/.keep
  apps/add-keys/out/foo.min.txt

# 2. Second run, test there was a cache hit (`cache` config`) and `output` was suppressed (`outputMode`)
  $ ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache hit, replaying output 78349dfba4d58116
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  add-keys:add-keys-task: cache hit, suppressing output c3b35d8aaef32d5b
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# 3. Change input file and assert cache miss
  $ echo "more text" >> $TARGET_DIR/apps/add-keys/src/foo.txt
  $ ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache miss, executing 1626b8684b31ff83
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing da7c8429d1c793ba
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo "running add-keys-task" > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# 4. Set env var and assert cache miss
  $ SOME_VAR=somevalue ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache hit, replaying output 1626b8684b31ff83
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing 72e64a10a351980c
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo "running add-keys-task" > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
