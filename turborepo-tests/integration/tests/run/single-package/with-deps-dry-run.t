Setup
  $ . ${TESTDIR}/../../../../helpers/setup.sh
  $ . ${TESTDIR}/../../_helpers/setup_monorepo.sh $(pwd) single_package

Check
  $ ${TURBO} run test --dry
  
  Global Hash Inputs
    Global Files                          = 3
    External Dependencies Hash            = 
    Global Cache Key                      = HEY STELLLLLLLAAAAAAAAAAAAA
    Global .env Files Considered          = 0
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
  
  Tasks to Run
  build
    Task                           = build                                                                                                                                           
    Hash                           = cb6df6cef2cfd596                                                                                                                                
    Cached (Local)                 = false                                                                                                                                           
    Cached (Remote)                = false                                                                                                                                           
    Command                        = echo 'building' > foo                                                                                                                           
    Outputs                        = foo                                                                                                                                             
    Log File                       = .turbo(\/|\\)turbo-build.log\s+ (re)
    Dependencies                   =                                                                                                                                                 
    Dependendents                  = test                                                                                                                                            
    Inputs Files Considered        = 5                                                                                                                                               
    .env Files Considered          = 0                                                                                                                                               
    Env Vars                       =                                                                                                                                                 
    Env Vars Values                =                                                                                                                                                 
    Inferred Env Vars Values       =                                                                                                                                                 
    Passed Through Env Vars        =                                                                                                                                                 
    Passed Through Env Vars Values =                                                                                                                                                 
    ResolvedTaskDefinition         = {"outputs":["foo.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","persistent":false,"env":[],"passThroughEnv":null,"dotEnv":null} 
    Framework                      = <NO FRAMEWORK DETECTED>                                                                                                                         
  test
    Task                           = test                                                                                                                                              
    Hash                           = f21d7ac37c171ce7                                                                                                                                  
    Cached (Local)                 = false                                                                                                                                             
    Cached (Remote)                = false                                                                                                                                             
    Command                        = cat foo.txt                                                                                                                                       
    Outputs                        =                                                                                                                                                   
    Log File                       = .turbo(\/|\\)turbo-test.log\s+ (re)
    Dependencies                   = build                                                                                                                                             
    Dependendents                  =                                                                                                                                                   
    Inputs Files Considered        = 5                                                                                                                                                 
    .env Files Considered          = 0                                                                                                                                                 
    Env Vars                       =                                                                                                                                                   
    Env Vars Values                =                                                                                                                                                   
    Inferred Env Vars Values       =                                                                                                                                                   
    Passed Through Env Vars        =                                                                                                                                                   
    Passed Through Env Vars Values =                                                                                                                                                   
    ResolvedTaskDefinition         = {"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputMode":"full","persistent":false,"env":[],"passThroughEnv":null,"dotEnv":null} 
    Framework                      = <NO FRAMEWORK DETECTED>                                                                                                                           
