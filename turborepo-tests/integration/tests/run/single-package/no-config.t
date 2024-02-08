Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files                          = 2\s* (re)
    External Dependencies Hash            =\s* (re)
    Global Cache Key                      = HEY STELLLLLLLAAAAAAAAAAAAA\s* (re)
    Global .env Files Considered          = 0\s* (re)
    Global Env Vars                       =\s* (re)
    Global Env Vars Values                =\s* (re)
    Inferred Global Env Vars Values       =\s* (re)
    Global Passed Through Env Vars        =\s* (re)
    Global Passed Through Env Vars Values =\s* (re)
  
  Tasks to Run
  build
    Task                           = build\s* (re)
    Hash                           = e46d6df5143cae99\s* (re)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = echo building > foo.txt\s* (re)
    Outputs                        =\s* (re)
    Log File                       = .turbo(\/|\\)turbo-build.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 4\s* (re)
    .env Files Considered          = 0\s* (re)
    Env Vars                       =\s* (re)
    Env Vars Values                =\s* (re)
    Inferred Env Vars Values       =\s* (re)
    Passed Through Env Vars        =\s* (re)
    Passed Through Env Vars Values =\s* (re)
    Resolved Task Definition       = {"outputs":\[],"cache":false,"dependsOn":\[],"inputs":\[],"outputMode":"full","persistent":false,"env":\[],"passThroughEnv":null,"dotEnv":null}\s* (re)
    Framework                      =\s* (re)

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
  build: cache bypass, force executing e46d6df5143cae99
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
  build: cache bypass, force executing e46d6df5143cae99
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  