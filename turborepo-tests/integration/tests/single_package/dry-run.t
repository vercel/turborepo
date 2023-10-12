Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package

Check
  $ ${TURBO} run build --dry
  
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
    Hash                           = 414559dcbbf03c41                                                                                                                                
    Cached (Local)                 = false                                                                                                                                           
    Cached (Remote)                = false                                                                                                                                           
    Command                        = echo 'building' > foo                                                                                                                           
    Outputs                        = foo                                                                                                                                             
    Log File                       = .turbo/turbo-build.log                                                                                                                          
    Dependencies                   =                                                                                                                                                 
    Dependendents                  =                                                                                                                                                 
    Inputs Files Considered        = 5                                                                                                                                               
    .env Files Considered          = 0                                                                                                                                               
    Env Vars                       =                                                                                                                                                 
    Env Vars Values                =                                                                                                                                                 
    Inferred Env Vars Values       =                                                                                                                                                 
    Passed Through Env Vars        =                                                                                                                                                 
    Passed Through Env Vars Values =                                                                                                                                                 
    ResolvedTaskDefinition         = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","persistent":false,"env":[],"passThroughEnv":null,"dotEnv":null} 
    Framework                      = <NO FRAMEWORK DETECTED>                                                                                                                         
