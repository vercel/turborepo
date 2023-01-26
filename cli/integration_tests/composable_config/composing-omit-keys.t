Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# The omit-keys-task in the root turbo.json has ALL the config. The workspace config
# defines the task, but does not override any of the keys. This test:
# [ ] Tests dependsOn works by asserting that another task runs first
# [ ] Tests outputs works by asserting that the right directory is cached
# [ ] Tests outputMode by asserting output logs on a second run
# [ ] Tests inputs works by changing a file and testing there was a cache miss
# [ ] Tests env works by setting an env var and asserting there was a cache miss

# 1. First run, assert for `dependsOn` and `outputs` keys
  $ ${TURBO} run omit-keys-task --skip-infer --filter=omit-keys -vvv > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: omit-keys (esc)
  \xe2\x80\xa2 Running omit-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  omit-keys:omit-keys-task: cache miss, executing 5c7fd63a64a3e13f
  omit-keys:omit-keys-task: 
  omit-keys:omit-keys-task: > omit-keys-task
  omit-keys:omit-keys-task: > echo "running omit-keys-task" > out/foo.min.txt
  omit-keys:omit-keys-task: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ HASH=$(cat tmp.log | grep -E "omit-keys:omit-keys-task.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/omit-keys/.turbo/turbo-omit-keys-task.log
  apps/omit-keys/out/
  apps/omit-keys/out/.keep
  apps/omit-keys/out/foo.min.txt

# 2. Second run, test there was a cache hit (`cache` config`) and `output` was suppressed (`outputMode`)
  $ ${TURBO} run omit-keys-task --skip-infer --filter=omit-keys
  \xe2\x80\xa2 Packages in scope: omit-keys (esc)
  \xe2\x80\xa2 Running omit-keys-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  omit-keys:omit-keys-task: cache hit, suppressing output 4704e217f779d371
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# 3. Change input file and assert cache miss
$ echo "more text" >> $TARGET_DIR/apps/omit-keys/src/foo.txt
$ ${TURBO} run omit-keys-task --skip-infer --filter=omit-keys
\xe2\x80\xa2 Packages in scope: omit-keys (esc)
\xe2\x80\xa2 Running omit-keys-task in 1 packages (esc)
\xe2\x80\xa2 Remote caching disabled (esc)
omit-keys:omit-keys-underlying-task: cache miss, executing 47f17f2be6e4f7d5
omit-keys:omit-keys-underlying-task: 
omit-keys:omit-keys-underlying-task: > omit-keys-underlying-task
omit-keys:omit-keys-underlying-task: > echo "running omit-keys-underlying-task"
omit-keys:omit-keys-underlying-task: 
omit-keys:omit-keys-underlying-task: running omit-keys-underlying-task
omit-keys:omit-keys-task: cache miss, executing a462cfd345a81245
omit-keys:omit-keys-task: 
omit-keys:omit-keys-task: > omit-keys-task
omit-keys:omit-keys-task: > echo "running omit-keys-task" > out/foo.min.txt
omit-keys:omit-keys-task: 

Tasks:    2 successful, 2 total
Cached:    0 cached, 2 total
Time:\s*[\.0-9]+m?s  (re)

# 4. Set env var and assert cache miss
$ SOME_VAR=somevalue ${TURBO} run omit-keys-task --skip-infer --filter=omit-keys
\xe2\x80\xa2 Packages in scope: omit-keys (esc)
\xe2\x80\xa2 Running omit-keys-task in 1 packages (esc)
\xe2\x80\xa2 Remote caching disabled (esc)
omit-keys:omit-keys-underlying-task: cache hit, replaying output 47f17f2be6e4f7d5
omit-keys:omit-keys-underlying-task: 
omit-keys:omit-keys-underlying-task: > omit-keys-underlying-task
omit-keys:omit-keys-underlying-task: > echo "running omit-keys-underlying-task"
omit-keys:omit-keys-underlying-task: 
omit-keys:omit-keys-underlying-task: running omit-keys-underlying-task
omit-keys:omit-keys-task: cache miss, executing 4842232c8296af30
omit-keys:omit-keys-task: 
omit-keys:omit-keys-task: > omit-keys-task
omit-keys:omit-keys-task: > echo "running omit-keys-task" > out/foo.min.txt
omit-keys:omit-keys-task: 

Tasks:    2 successful, 2 total
Cached:    1 cached, 2 total
Time:\s*[\.0-9]+m?s  (re)

