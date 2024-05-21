Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The override-values-task task in the root turbo.json has ALL the config. The workspace config
# defines the task and overrides all the keys. The tests below use `override-values-task` to assert that:
# - `outputs`, `inputs`, `env`, and `outputLogs` are overriden from the root config.

# 1. First run, assert that the right `outputs` are cached.
  $ ${TURBO} run override-values-task --filter=override-values > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache miss, executing 6c14afaa74bffc4a
=======
  override-values:override-values-task: cache miss, executing e52b31177ae7fb13
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ HASH=$(cat tmp.log | grep -E "override-values:override-values-task.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/.turbo/cache/$HASH.tar.zst;
  apps/override-values/.turbo/turbo-override-values-task.log
  apps/override-values/lib/
  apps/override-values/lib/.keep
  apps/override-values/lib/bar.min.txt

2. Run again and assert cache hit, and that full output is displayed
  $ ${TURBO} run override-values-task --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache hit, replaying logs 6c14afaa74bffc4a
=======
  override-values:override-values-task: cache hit, replaying logs e52b31177ae7fb13
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
3. Change input file and assert cache miss
  $ echo "more text" >> $TARGET_DIR/apps/override-values/src/bar.txt
  $ ${TURBO} run override-values-task --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache miss, executing 044ce2c43ae740f8
=======
  override-values:override-values-task: cache miss, executing e01ac3613896ebc3
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
3a. Change a file that is declared as input in root config, and assert cache hit and FULL TURBO
  $ echo "more text" >> $TARGET_DIR/apps/override-values/src/foo.txt
  $ ${TURBO} run override-values-task --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache hit, replaying logs 044ce2c43ae740f8
=======
  override-values:override-values-task: cache hit, replaying logs e01ac3613896ebc3
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
4. Set env var and assert cache miss, and that hash is different from above
  $ OTHER_VAR=somevalue ${TURBO} run override-values-task --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache miss, executing 961e6285b43ebb88
=======
  override-values:override-values-task: cache miss, executing d074022cd01831a2
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
4a. Set env var that is declared in root config, and assert cache hit and FULL TURBO
  $ OTHER_VAR=somevalue ${TURBO} run override-values-task --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  override-values:override-values-task: cache hit, replaying logs 961e6285b43ebb88
=======
  override-values:override-values-task: cache hit, replaying logs d074022cd01831a2
>>>>>>> 37c3c596f1 (chore: update integration tests)
  override-values:override-values-task: 
  override-values:override-values-task: > override-values-task
  override-values:override-values-task: > echo running-override-values-task > lib/bar.min.txt
  override-values:override-values-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
