Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files               = 3
    External Dependencies Hash = 
    Global Cache Key           = You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am.
  
  Tasks to Run
  build
    Task                             = build                                                                                                                                           
    Hash                             = dba2114627bfc5c1                                                                                                                                
    Cached (Local)                   = false                                                                                                                                           
    Cached (Remote)                  = false                                                                                                                                           
    Command                          = echo 'building' > foo                                                                                                                           
    Outputs                          = foo                                                                                                                                             
    Log File                         = .turbo/turbo-build.log                                                                                                                          
    Dependencies                     =                                                                                                                                                 
    Dependendents                    =                                                                                                                                                 
    Inputs Files Considered          = 5                                                                                                                                               
    .env Files Considered            = 0                                                                                                                                               
    Configured Environment Variables =                                                                                                                                                 
    Inferred Environment Variables   =                                                                                                                                                 
    Global Environment Variables     = VERCEL_ANALYTICS_ID=                                                                                                                            
    ResolvedTaskDefinition           = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","passThroughEnv":null,"dotEnv":null,"env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                                                         
