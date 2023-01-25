Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo
# depends-on--omit#build. dependsOn compile, but this dependence is inherited from root turbo.json
# because depends-on--omit turbo.json omits dependsOn key.
# Test that both compile and build are run in the right order.
  $ ${TURBO} run build --skip-infer --filter=depends-on--omit
  \xe2\x80\xa2 Packages in scope: depends-on--omit (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  depends-on--omit:compile: cache miss, executing a1a9791d07a45ce3
  depends-on--omit:compile: 
  depends-on--omit:compile: > compile
  depends-on--omit:compile: > echo "compiling in depends-on--omit"
  depends-on--omit:compile: 
  depends-on--omit:compile: compiling in depends-on--omit
  depends-on--omit:build: cache miss, executing 5bde766b94f6d47e
  depends-on--omit:build: 
  depends-on--omit:build: > build
  depends-on--omit:build: > echo "building depends-on--omit" > lib/foo.txt && echo "building depends-on--omit" > out/foo.txt
  depends-on--omit:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  

# depends-on--add-key#build. dependsOn compile, which depends on precompile.
# Root turbo.json depends on compile, but compile does not depend on anything.
# Test that all three tasks are executed in order.
  $ ${TURBO} run build --skip-infer --filter=depends-on--add-key
  \xe2\x80\xa2 Packages in scope: depends-on--add-key (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  depends-on--add-key:beforecompile: cache miss, executing b490f8b405451007
  depends-on--add-key:beforecompile: 
  depends-on--add-key:beforecompile: > beforecompile
  depends-on--add-key:beforecompile: > echo "beforecompiling in depends-on--add-key"
  depends-on--add-key:beforecompile: 
  depends-on--add-key:beforecompile: beforecompiling in depends-on--add-key
  depends-on--add-key:compile: cache miss, executing 90185972d311b0fc
  depends-on--add-key:compile: 
  depends-on--add-key:compile: > compile
  depends-on--add-key:compile: > echo "compiling in depends-on--add-key"
  depends-on--add-key:compile: 
  depends-on--add-key:compile: compiling in depends-on--add-key
  depends-on--add-key:build: cache miss, executing 67f220d153f76374
  depends-on--add-key:build: 
  depends-on--add-key:build: > build
  depends-on--add-key:build: > echo "building depends-on--add-key" > lib/foo.txt && echo "building depends-on--add-key" > out/foo.txt
  depends-on--add-key:build: 
  
   Tasks:    3 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  