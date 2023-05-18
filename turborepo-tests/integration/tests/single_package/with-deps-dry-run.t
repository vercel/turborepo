Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package_deps

Check
  $ ${TURBO} run test --dry
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
  
  Tasks to Run
  build
    Task                             = build                                                                                                                             
    Hash                             = 1ef465607dcd4131                                                                                                                  
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
    Hash                             = c661131fb01cd243                                                                                                                    
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
