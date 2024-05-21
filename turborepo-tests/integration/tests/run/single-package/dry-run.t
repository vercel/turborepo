Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run build --dry
  
  Global Hash Inputs
    Global Files                          = 3
    External Dependencies Hash            = 
<<<<<<< HEAD
    Global Cache Key                      = HEY STELLLLLLLAAAAAAAAAAAAA
=======
    Global Cache Key                      = I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here (esc)
    Global .env Files Considered          = 0
>>>>>>> 37c3c596f1 (chore: update integration tests)
    Global Env Vars                       = 
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 
  
  Tasks to Run
  build
    Task                           = build\s* (re)
<<<<<<< HEAD
    Hash                           = fbef1dba65f21ba4
=======
    Hash                           = b69493e87ea97b0e
>>>>>>> 37c3c596f1 (chore: update integration tests)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Command                        = echo building > foo.txt\s* (re)
    Outputs                        = foo.txt\s* (re)
    Log File                       = .turbo(\/|\\)turbo-build.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 5\s* (re)
    Env Vars                       = 
    Env Vars Values                = 
    Inferred Env Vars Values       = 
    Passed Through Env Vars        = 
    Passed Through Env Vars Values = 
    Resolved Task Definition       = {"outputs":["foo.txt"],"cache":true,"dependsOn":[],"inputs":[],"outputLogs":"full","persistent":false,"env":[],"passThroughEnv":null,"interactive":false}
    Framework                      = 
