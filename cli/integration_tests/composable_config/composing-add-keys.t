Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# TMP DEBUG:
# "dependsOn": ["add-key-underlying-task"],
# "inputs": ["foo.txt"]
# "outputs": ["out/**"]
# "env": ["SOME_VAR"]
# "outputMode": "new-only"

# The add-keys-task in the root turbo.json has no config
# [x] Test dependOn works by testing that output runs another task
# [x] Test outputs works by testing that the right directory is cached
# Test outputMode by checking output
# Test inputs works by changing a file and testing there was a cache miss
# Test env works by setting an env var and asserting there was a cache miss

# 1. First run, assert for `dependsOn` and `outputs` keys
  $ ${TURBO} run add-keys-task --skip-infer --filter=add-keys > tmp.log
  Hashing error: cannot find package-file hash for add-keys#src/foo.txt
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache miss, executing 0809e4def0bb1de2
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  add-keys:add-keys-task: cache miss, executing 
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
# 2. Second run, test there was a cache hit and output was suppressed, because of outputMode
  $ ${TURBO} run add-keys-task --skip-infer --filter=add-keys
  \xe2\x80\xa2 Packages in scope: add-keys (esc)
  \xe2\x80\xa2 Running add-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-keys:add-keys-underlying-task: cache hit, replaying output 0809e4def0bb1de2
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: > add-keys-underlying-task
  add-keys:add-keys-underlying-task: > echo "running add-keys-underlying-task"
  add-keys:add-keys-underlying-task: 
  add-keys:add-keys-underlying-task: running add-keys-underlying-task
  Hashing error: cannot find package-file hash for add-keys#src/foo.txt
  add-keys:add-keys-task: cache hit, suppressing output 
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# 3. Change input file and assert cache miss
# 4. Set env var and assert cache miss
