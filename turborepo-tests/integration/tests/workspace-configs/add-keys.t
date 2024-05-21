Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The add-keys-task in the root turbo.json has no config. This test:
# [x] Tests dependsOn works by asserting that another task runs first
# [x] Tests outputs works by asserting that the right directory is cached
# [x] Tests outputLogs by asserting output logs on a second run
# [x] Tests inputs works by changing a file and testing there was a cache miss
# [x] Tests env works by setting an env var and asserting there was a cache miss

# 1. First run, assert for `dependsOn` and `outputs` keys
  $ ${TURBO} run add-keys-task --filter=add-keys > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache miss, executing b1b44ef21c97573a
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo running-add-keys-underlying-task
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running-add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing 900c42a3c99fcb70
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo running-add-keys-task > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
<<<<<<< HEAD
=======
>>>>>>> 37c3c596f1 (chore: update integration tests)
<<<<<<< HEAD
=======
>>>>>>> 37c3c596f1 (chore: update integration tests)
  $ HASH=$(cat tmp.log | grep -E "add-keys:add-keys-task.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/.turbo/cache/$HASH.tar.zst;
  apps/add-keys/.turbo/turbo-add-keys-task.log
  apps/add-keys/out/
  apps/add-keys/out/.keep
  apps/add-keys/out/foo.min.txt

# 2. Second run, test there was a cache hit (`cache` config`) and `output` was suppressed (`outputLogs`)
  $ ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache hit, replaying logs b1b44ef21c97573a
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo running-add-keys-underlying-task
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running-add-keys-underlying-task
  add-keys:add-keys-task: cache hit, suppressing logs 900c42a3c99fcb70
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# 3. Change input file and assert cache miss
  $ echo "more text" >> $TARGET_DIR/apps/add-keys/src/foo.txt
  $ ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache miss, executing 8e33b91a6ca8fcd4
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo running-add-keys-underlying-task
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running-add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing c99ca18c10380fb8
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo running-add-keys-task > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# 4. Set env var and assert cache miss
  $ SOME_VAR=somevalue ${TURBO} run add-keys-task --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache hit, replaying logs 8e33b91a6ca8fcd4
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo running-add-keys-underlying-task
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running-add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing de9692b5d6414208
  add-keys:add-keys-task: 
  add-keys:add-keys-task: > add-keys-task
  add-keys:add-keys-task: > echo running-add-keys-task > out/foo.min.txt
  add-keys:add-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
