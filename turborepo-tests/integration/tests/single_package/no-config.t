Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package
  $ rm turbo.json
  $ git commit -am "Delete turbo config" --quiet

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"//#build":{"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false}}
  
  Tasks to Run
  build
    Task                             = build                                                                                                   
    Hash                             = 5b5ae44052e3d624                                                                                        
    Cached (Local)                   = false                                                                                                   
    Cached (Remote)                  = false                                                                                                   
    Command                          = echo 'building' > foo                                                                                   
    Outputs                          =                                                                                                         
    Log File                         = .turbo/turbo-build.log                                                                                  
    Dependencies                     =                                                                                                         
    Dependendents                    =                                                                                                         
    Inputs Files Considered          = 4                                                                                                       
    Configured Environment Variables =                                                                                                         
    Inferred Environment Variables   =                                                                                                         
    Global Environment Variables     = VERCEL_ANALYTICS_ID=                                                                                    
    ResolvedTaskDefinition           = {"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                 

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
  build: cache bypass, force executing 5b5ae44052e3d624
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
  build: cache bypass, force executing 5b5ae44052e3d624
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  