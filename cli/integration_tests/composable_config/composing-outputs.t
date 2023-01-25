Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# outputs--omit-key#build. Writes files to lib/ and out/, but only out/ is cached via the root turbo.json outputs key.
# There is no workspace turbo.json, so we should expect the behavior defined in the root.
  $ ${TURBO} run build --skip-infer --filter=outputs--omit-key > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: outputs--omit-key (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  outputs--omit-key:compile: cache miss, executing 43198787d785b53c
  outputs--omit-key:compile: 
  outputs--omit-key:compile: > compile
  outputs--omit-key:compile: > echo "compiling in outputs--omit-key"
  outputs--omit-key:compile: 
  outputs--omit-key:compile: compiling in outputs--omit-key
  outputs--omit-key:build: cache miss, executing d15789834ad4ad62
  outputs--omit-key:build: 
  outputs--omit-key:build: > build
  outputs--omit-key:build: > echo "building outputs--omit-key" > lib/foo.txt && echo "building outputs--omit-key" > out/foo.txt
  outputs--omit-key:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Look in the saved logs for the hash, so we can inspect the tarball with the same name
  $ HASH=$(cat tmp.log | grep -E "outputs--omit-key:build.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/outputs--omit-key/.turbo/turbo-build.log
  apps/outputs--omit-key/out/
  apps/outputs--omit-key/out/.keep
  apps/outputs--omit-key/out/foo.txt

# outputs--override-value#build. outputs to both lib/ and out/ directories but only lib/ is cached
# Saves log output to a file so we can fish out the task hash, and then inspect the cached tarball.
  $ ${TURBO} run build --skip-infer --filter=outputs--override-value > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: outputs--override-value (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  outputs--override-value:compile: cache miss, executing 433f071a3d1da002
  outputs--override-value:compile: 
  outputs--override-value:compile: > compile
  outputs--override-value:compile: > echo "compiling in outputs--override-value"
  outputs--override-value:compile: 
  outputs--override-value:compile: compiling in outputs--override-value
  outputs--override-value:build: cache miss, executing 1d4219b2d5415ad0
  outputs--override-value:build: 
  outputs--override-value:build: > build
  outputs--override-value:build: > echo "building outputs--override-value" > lib/foo.txt && echo "building outputs--override-value" > out/foo.txt
  outputs--override-value:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Look in the saved logs for the hash, so we can inspect the tarball with the same name
  $ HASH=$(cat tmp.log | grep -E "outputs--override-value:build.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/outputs--override-value/.turbo/turbo-build.log
  apps/outputs--override-value/lib/
  apps/outputs--override-value/lib/.keep
  apps/outputs--override-value/lib/foo.txt

