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
    Engines Values                        = 
  
  Tasks to Run
  build
    Task                           = build\s* (re)
    Hash                           = fe0059df5e6291b2
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = echo building > foo.txt\s* (re)
    Outputs                        = foo.txt\s* (re)
    Log File                       = .turbo(\/|\\)turbo-build.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     = test\s* (re)
    With                           =\s* (re)
    Inputs Files Considered        = 5\s* (re)
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":["foo.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputLogs":"full","persistent":false,"interruptible":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 
  test
    Task                           = test\s* (re)
    Hash                           = 7cfbd8e30495d802
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = cat foo.txt\s* (re)
    Outputs                        =\s* (re)
    Log File                       = .turbo(\/|\\)turbo-test.log\s* (re)
    Dependencies                   = build\s* (re)
    Dependents                     =\s* (re)
    With                           =\s* (re)
    Inputs Files Considered        = 5\s* (re)
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":[],"cache":true,"dependsOn":["build"],"inputs":[],"outputLogs":"full","persistent":false,"interruptible":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 
