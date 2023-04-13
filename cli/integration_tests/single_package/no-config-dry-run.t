Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
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
