Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files                          = 2\s* (re)
    External Dependencies Hash            =\s* (re)
    Global Cache Key                      = I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here (esc)
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
  
  Tasks to Run
  build
    Task                           = build\s* (re)
    Hash                           = a6da7b8ddbe2bb84
    Cached (Local)                 = false
    Cached (Remote)                = false
    Command                        = echo building > foo.txt
    Outputs                        = 
    Log File                       = .turbo/turbo-build.log
    Dependencies                   = 
    Dependents                     = 
    Inputs Files Considered        = 4
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
  build: cache bypass, force executing a6da7b8ddbe2bb84
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
  build: cache bypass, force executing a6da7b8ddbe2bb84
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  