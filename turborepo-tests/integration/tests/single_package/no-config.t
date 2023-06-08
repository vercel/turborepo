Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files                          = 2
    External Dependencies Hash            = 
    Global Cache Key                      = You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am.
    Global .env Files Considered          = 0
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
  
  Tasks to Run
  build
    Task                           = build                                                                                                                                       
    Hash                           = a75d75c904c562c5                                                                                                                            
    Cached (Local)                 = false                                                                                                                                       
    Cached (Remote)                = false                                                                                                                                       
    Command                        = echo 'building' > foo                                                                                                                       
    Outputs                        =                                                                                                                                             
    Log File                       = .turbo/turbo-build.log                                                                                                                      
    Dependencies                   =                                                                                                                                             
    Dependendents                  =                                                                                                                                             
    Inputs Files Considered        = 4                                                                                                                                           
    .env Files Considered          = 0                                                                                                                                           
    Env Vars                       =                                                                                                                                             
    Env Vars Values                =                                                                                                                                             
    Inferred Env Vars Values       =                                                                                                                                             
    Passed Through Env Vars        =                                                                                                                                             
    Passed Through Env Vars Values =                                                                                                                                             
    ResolvedTaskDefinition         = {"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","persistent":false,"env":[],"passThroughEnv":null,"dotEnv":null} 
    Framework                      = <NO FRAMEWORK DETECTED>                                                                                                                     

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
  build: cache bypass, force executing a75d75c904c562c5
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify no caching because there is no config
  $ ${TURBO} run build --single-package
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache bypass, force executing a75d75c904c562c5
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  