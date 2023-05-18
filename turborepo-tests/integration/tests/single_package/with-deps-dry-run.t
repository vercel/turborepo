Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package_deps

Check
  $ ${TURBO} run test --dry
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am.
  
  Tasks to Run
  build
    Task                             = build                                                                                                                             
    Hash                             = 0c92d3f6d192dce0                                                                                                                  
    Cached (Local)                   = false                                                                                                                             
    Cached (Remote)                  = false                                                                                                                             
    Command                          = echo 'building' > foo                                                                                                             
    Outputs                          = foo                                                                                                                               
    Log File                         = .turbo/turbo-build.log                                                                                                            
    Dependencies                     =                                                                                                                                   
    Dependendents                    = test                                                                                                                              
    Inputs Files Considered          = 4                                                                                                                                 
    Configured Environment Variables =                                                                                                                                   
    Inferred Environment Variables   =                                                                                                                                   
    Global Environment Variables     = VERCEL_ANALYTICS_ID=                                                                                                              
    ResolvedTaskDefinition           = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","passThroughEnv":null,"env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                                           
  test
    Task                             = test                                                                                                                                
    Hash                             = d204b0e91f222208                                                                                                                    
    Cached (Local)                   = false                                                                                                                               
    Cached (Remote)                  = false                                                                                                                               
    Command                          = [[ ( -f foo ) && $(cat foo) == 'building' ]]                                                                                        
    Outputs                          =                                                                                                                                     
    Log File                         = .turbo/turbo-test.log                                                                                                               
    Dependencies                     = build                                                                                                                               
    Dependendents                    =                                                                                                                                     
    Inputs Files Considered          = 4                                                                                                                                   
    Configured Environment Variables =                                                                                                                                     
    Inferred Environment Variables   =                                                                                                                                     
    Global Environment Variables     = VERCEL_ANALYTICS_ID=                                                                                                                
    ResolvedTaskDefinition           = {"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputMode":"full","passThroughEnv":null,"env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                                             
