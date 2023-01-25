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
  depends-on--omit:compile: cache miss, executing a03840dc6b8a343b
  depends-on--omit:compile: 
  depends-on--omit:compile: > compile
  depends-on--omit:compile: > echo "compiling in depends-on--omit"
  depends-on--omit:compile: 
  depends-on--omit:compile: compiling in depends-on--omit
  depends-on--omit:build: cache miss, executing 755e4c2418d7a217
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
  


# depends-on--override#build. dependsOn compile, which depends on precompile.
# Root turbo.json depends on compile, but compile does not depend on anything.
# Test that all three tasks are executed in order.
  $ ${TURBO} run build --skip-infer --filter=depends-on--override
  \xe2\x80\xa2 Packages in scope: depends-on--override (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  depends-on--override:somethingelse: cache miss, executing 75bbade08c42dd53
  depends-on--override:somethingelse: 
  depends-on--override:somethingelse: > somethingelse
  depends-on--override:somethingelse: > echo "somethingelse depends-on--override"
  depends-on--override:somethingelse: 
  depends-on--override:somethingelse: somethingelse depends-on--override
  depends-on--override:build: cache miss, executing a90407b3f729eeb7
  depends-on--override:build: 
  depends-on--override:build: > build
  depends-on--override:build: > echo "building depends-on--override" > lib/foo.txt && echo "building depends-on--override" > out/foo.txt
  depends-on--override:build: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  