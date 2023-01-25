Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# outputs--omit#build. Writes files to lib/ and out/, but only out/ is cached via the root turbo.json outputs key.
# There is no workspace turbo.json, so we should expect the behavior defined in the root.
  $ ${TURBO} run build --skip-infer --filter=outputs--omit > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: outputs--omit (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  outputs--omit:compile: cache miss, executing 7fe592d69421dd8a
  outputs--omit:compile: 
  outputs--omit:compile: > compile
  outputs--omit:compile: > echo "compiling in outputs--omit"
  outputs--omit:compile: 
  outputs--omit:compile: compiling in outputs--omit
  outputs--omit:build: cache miss, executing d373f6083d8d62be
  outputs--omit:build: 
  outputs--omit:build: > build
  outputs--omit:build: > echo "building outputs--omit" > lib/foo.txt && echo "building outputs--omit" > out/foo.txt
  outputs--omit:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Look in the saved logs for the hash, so we can inspect the tarball with the same name
  $ HASH=$(cat tmp.log | grep -E "outputs--omit:build.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/outputs--omit/.turbo/turbo-build.log
  apps/outputs--omit/out/
  apps/outputs--omit/out/.keep
  apps/outputs--omit/out/foo.txt

# outputs--override-key#build. outputs to both lib/ and out/ directories but only lib/ is cached
# Saves log output to a file so we can fish out the task hash, and then inspect the cached tarball.
  $ ${TURBO} run build --skip-infer --filter=outputs--override-key > tmp.log
  $ cat tmp.log
  \xe2\x80\xa2 Packages in scope: outputs--override-key (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  outputs--override-key:compile: cache miss, executing 244f977396f20945
  outputs--override-key:compile: 
  outputs--override-key:compile: > compile
  outputs--override-key:compile: > echo "compiling in outputs--override-key"
  outputs--override-key:compile: 
  outputs--override-key:compile: compiling in outputs--override-key
  outputs--override-key:build: cache miss, executing 3554f31ac076b388
  outputs--override-key:build: 
  outputs--override-key:build: > build
  outputs--override-key:build: > echo "building outputs--override-key" > lib/foo.txt && echo "building outputs--override-key" > out/foo.txt
  outputs--override-key:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Look in the saved logs for the hash, so we can inspect the tarball with the same name
  $ HASH=$(cat tmp.log | grep -E "outputs--override-key:build.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/node_modules/.cache/turbo/$HASH.tar.zst;
  apps/outputs--override-key/.turbo/turbo-build.log
  apps/outputs--override-key/lib/
  apps/outputs--override-key/lib/.keep
  apps/outputs--override-key/lib/foo.txt

