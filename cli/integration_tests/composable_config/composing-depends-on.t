Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo
# depends-on--omit-key#build. dependsOn compile, but this dependence is inherited from root turbo.json
# because depends-on--omit-key turbo.json omits dependsOn key.
# Test that both compile and build are run in the right order.
  $ ${TURBO} run build --skip-infer --filter=depends-on--omit-key
  \xe2\x80\xa2 Packages in scope: depends-on--omit-key (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  depends-on--omit-key:compile: cache miss, executing 97fe079b4bc13291
  depends-on--omit-key:compile: 
  depends-on--omit-key:compile: > compile
  depends-on--omit-key:compile: > echo "compiling in depends-on--omit-key"
  depends-on--omit-key:compile: 
  depends-on--omit-key:compile: compiling in depends-on--omit-key
  depends-on--omit-key:build: cache miss, executing b204975bd2ef2c43
  depends-on--omit-key:build: 
  depends-on--omit-key:build: > build
  depends-on--omit-key:build: > echo "building depends-on--omit-key" > lib/foo.txt && echo "building depends-on--omit-key" > out/foo.txt
  depends-on--omit-key:build: 
  
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
  depends-on--add-key:beforecompile: cache miss, executing 26238dca0919534a
  depends-on--add-key:beforecompile: 
  depends-on--add-key:beforecompile: > beforecompile
  depends-on--add-key:beforecompile: > echo "beforecompiling in depends-on--add-key"
  depends-on--add-key:beforecompile: 
  depends-on--add-key:beforecompile: beforecompiling in depends-on--add-key
  depends-on--add-key:compile: cache miss, executing 99a2b2e2ffcc20ea
  depends-on--add-key:compile: 
  depends-on--add-key:compile: > compile
  depends-on--add-key:compile: > echo "compiling in depends-on--add-key"
  depends-on--add-key:compile: 
  depends-on--add-key:compile: compiling in depends-on--add-key
  depends-on--add-key:build: cache miss, executing 79812c9248e6cf92
  depends-on--add-key:build: 
  depends-on--add-key:build: > build
  depends-on--add-key:build: > echo "building depends-on--add-key" > lib/foo.txt && echo "building depends-on--add-key" > out/foo.txt
  depends-on--add-key:build: 
  
   Tasks:    3 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  

# depends-on--override-value#build. dependsOn compile, which depends on precompile.
# Root turbo.json depends on compile, but compile does not depend on anything.
# Test that all three tasks are executed in order.
  $ ${TURBO} run build --skip-infer --filter=depends-on--override-value
  \xe2\x80\xa2 Packages in scope: depends-on--override-value (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  depends-on--override-value:somethingelse: cache miss, executing e119e2fc2adf5146
  depends-on--override-value:somethingelse: 
  depends-on--override-value:somethingelse: > somethingelse
  depends-on--override-value:somethingelse: > echo "somethingelse depends-on--override-value"
  depends-on--override-value:somethingelse: 
  depends-on--override-value:somethingelse: somethingelse depends-on--override-value
  depends-on--override-value:build: cache miss, executing 60eb153849befc65
  depends-on--override-value:build: 
  depends-on--override-value:build: > build
  depends-on--override-value:build: > echo "building depends-on--override-value" > lib/foo.txt && echo "building depends-on--override-value" > out/foo.txt
  depends-on--override-value:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  