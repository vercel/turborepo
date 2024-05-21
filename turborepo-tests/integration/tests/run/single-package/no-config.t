Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files                          = 2\s* (re)
    External Dependencies Hash            =\s* (re)
<<<<<<< HEAD
    Global Cache Key                      = HEY STELLLLLLLAAAAAAAAAAAAA\s* (re)
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
=======
    Global Cache Key                      = I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here (esc)
    Global .env Files Considered          = 0\s* (re)
    Global Env Vars                       =\s* (re)
    Global Env Vars Values                =\s* (re)
    Inferred Global Env Vars Values       =\s* (re)
    Global Passed Through Env Vars        =\s* (re)
    Global Passed Through Env Vars Values =\s* (re)
>>>>>>> 2eae5cbd82 (Update tests)
  
  Tasks to Run
  build
    Task                           = build\s* (re)
<<<<<<< HEAD
<<<<<<< HEAD
    Hash                           = 10229b8c4ed48f95
=======
    Hash                           = 64c6a531cb00bb47
>>>>>>> 2eae5cbd82 (Update tests)
=======
    Hash                           = 9cff709a1e22ead6
>>>>>>> 37c3c596f1 (chore: update integration tests)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = echo building > foo.txt\s* (re)
    Outputs                        =\s* (re)
    Log File                       = .turbo(\/|\\)turbo-build.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 4\s* (re)
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputLogs":"full","persistent":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 

  $ ${TURBO} run build --graph
  
  digraph {
  \tcompound = "true" (esc)
  \tnewrank = "true" (esc)
  \tsubgraph "root" { (esc)
  \t\t"[root] build" -> "[root] ___ROOT___" (esc)
  \t} (esc)
  }
  
Run real once
  $ ${TURBO} run build
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
<<<<<<< HEAD
  build: cache bypass, force executing 10229b8c4ed48f95
=======
  build: cache bypass, force executing 64c6a531cb00bb47
>>>>>>> 2eae5cbd82 (Update tests)
=======
  build: cache bypass, force executing 9cff709a1e22ead6
>>>>>>> 37c3c596f1 (chore: update integration tests)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify no caching because there is no config
  $ ${TURBO} run build
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
<<<<<<< HEAD
  build: cache bypass, force executing 10229b8c4ed48f95
=======
  build: cache bypass, force executing 64c6a531cb00bb47
>>>>>>> 2eae5cbd82 (Update tests)
=======
  build: cache bypass, force executing 9cff709a1e22ead6
>>>>>>> 37c3c596f1 (chore: update integration tests)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  