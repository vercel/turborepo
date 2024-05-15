Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

# Run the build task with --dry flag and cut up the logs into separate files by empty lines
# https://stackoverflow.com/a/33297878/986415
  $ ${TURBO} run build --dry |  awk -v RS= '{print > ("tmp-" NR ".txt")}'

# The first part of the file is Packages in Scope
  $ cat tmp-1.txt
  Packages in Scope
  Name    Path\s* (re)
  another packages(\/|\\)another\s* (re)
  my-app  apps(\/|\\)my-app\s* (re)
  util    packages(\/|\\)util\s* (re)

# Part 2 of the logs are Global Hash INputs
  $ cat tmp-2.txt
  Global Hash Inputs
    Global Files                          = 1
    External Dependencies Hash            = 459c029558afe716
<<<<<<< HEAD
    Global Cache Key                      = HEY STELLLLLLLAAAAAAAAAAAAA
=======
    Global Cache Key                      = I can\xe2\x80\x99t see ya, but I know you\xe2\x80\x99re here (esc)
    Global .env Files Considered          = 0
>>>>>>> 2eae5cbd82 (Update tests)
    Global Env Vars                       = SOME_ENV_VAR
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 

# Part 3 are Tasks to Run, and we have to validate each task separately
  $ cat tmp-3.txt | grep "my-app#build" -A 17
  my-app#build
    Task                           = build\s* (re)
    Package                        = my-app\s* (re)
<<<<<<< HEAD
    Hash                           = 61394a550211cbe8\s* (re)
=======
    Hash                           = dea27cc2fbbcfe2c
>>>>>>> 2eae5cbd82 (Update tests)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Directory                      = apps(\/|\\)my-app\s* (re)
    Command                        = echo building\s* (re)
    Outputs                        = apple.json, banana.txt\s* (re)
    Log File                       = apps(\/|\\)my-app(\/|\\)\.turbo(\/|\\)turbo-build\.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 2\s* (re)
    Env Vars                       =\s* (re)
    Env Vars Values                =\s* (re)
    Inferred Env Vars Values       =\s* (re)
    Passed Through Env Vars        =\s* (re)
    Passed Through Env Vars Values =\s* (re)

  $ cat tmp-3.txt | grep "util#build" -A 17
  util#build
    Task                           = build\s* (re)
    Package                        = util\s* (re)
<<<<<<< HEAD
    Hash                           = d30fc4474534c30e\s* (re)
=======
    Hash                           = e09943c27ed0a75d
>>>>>>> 2eae5cbd82 (Update tests)
    Cached \(Local\)                 = false\s* (re)
    Cached \(Remote\)                = false\s* (re)
    Directory                      = packages(\/|\\)util\s* (re)
    Command                        = echo building\s* (re)
    Outputs                        =\s* (re)
    Log File                       = packages(\/|\\)util(\/|\\)\.turbo(\/|\\)turbo-build\.log\s* (re)
    Dependencies                   =\s* (re)
    Dependents                     =\s* (re)
    Inputs Files Considered        = 1\s* (re)
    Env Vars                       = NODE_ENV\s* (re)
    Env Vars Values                =\s* (re)
    Inferred Env Vars Values       =\s* (re)
    Passed Through Env Vars        =\s* (re)
    Passed Through Env Vars Values =\s* (re)

# Run the task with NODE_ENV set and see it in summary. Use util package so it's just one package
  $ NODE_ENV=banana ${TURBO} run build --dry --filter=util | grep "Environment Variables"
  [1]
