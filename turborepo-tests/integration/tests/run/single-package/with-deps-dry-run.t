Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

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
    Task                           = build\s* (re)
    Hash                           = f09bf783beacf5c9\s* (re)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = echo building > foo.txt\s* (re)
    Outputs                        = foo.txt\s* (re)
    Log File                       = .turbo(\/|\\)turbo-build.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     = test\s* (re)
    Inputs Files Considered        = 5\s* (re)
    .env Files Considered          = 0\s* (re)
    Env Vars                       =\s* (re)
    Env Vars Values                =\s* (re)
    Inferred Env Vars Values       =\s* (re)
    Passed Through Env Vars        =\s* (re)
    Passed Through Env Vars Values =\s* (re)
    Resolved Task Definition       = {"outputs":\["foo.txt"],"cache":true,"dependsOn":\[],"inputs":\[],"outputMode":"full","persistent":false,"env":\[],"passThroughEnv":null,"dotEnv":null}\s* (re)
    Framework                      =\s* (re)
  test
    Task                           = test\s* (re)
    Hash                           = 8bfab5dc6b4ccb3b\s* (re)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = cat foo.txt\s* (re)
    Outputs                        =\s* (re)
    Log File                       = .turbo(\/|\\)turbo-test.log\s* (re)
    Dependencies                   = build\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 5\s* (re)
    .env Files Considered          = 0\s* (re)
    Env Vars                       =\s* (re)
    Env Vars Values                =\s* (re)
    Inferred Env Vars Values       =\s* (re)
    Passed Through Env Vars        =\s* (re)
    Passed Through Env Vars Values =\s* (re)
    Resolved Task Definition       = {"outputs":\[],"cache":true,"dependsOn":\["build"],"inputs":\[],"outputMode":"full","persistent":false,"env":\[],"passThroughEnv":null,"dotEnv":null}\s* (re)
    Framework                      =\s* (re)
