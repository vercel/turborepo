Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run test --dry
  
  Global Hash Inputs
    Global Files                          = 3
    External Dependencies Hash            = 
    Global Cache Key                      = I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here (esc)
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
  
  Tasks to Run
  build
    Task                           = build\s* (re)
    Hash                           = 81a933c332d3f388
    Cached (Local)                 = false
    Cached (Remote)                = false
    Command                        = echo building > foo.txt
    Outputs                        = foo.txt
    Log File                       = .turbo(\/|\\)turbo-build.log (re)
    Dependencies                   = 
    Dependents                     = test
    Inputs Files Considered        = 5
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":["foo.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputLogs":"full","persistent":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 
  test
    Task                           = test\s* (re)
    Hash                           = 785d8ef1115bde3b
    Cached (Local)                 = false
    Cached (Remote)                = false
    Command                        = cat foo.txt
    Outputs                        = 
    Log File                       = .turbo(\/|\\)turbo-test.log (re)
    Dependencies                   = build
    Dependents                     = 
    Inputs Files Considered        = 5
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputLogs":"full","persistent":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 
