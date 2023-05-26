Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

# Run the build task with --dry flag and cut up the logs into separate files by empty lines
# https://stackoverflow.com/a/33297878/986415
  $ ${TURBO} run build --dry |  awk -v RS= '{print > ("tmp-" NR ".txt")}'

# The first part of the file is Packages in Scope
  $ cat tmp-1.txt
  Packages in Scope
  Name    Path             
  another packages/another 
  my-app  apps/my-app      
  util    packages/util    

# Part 2 of the logs are Global Hash INputs
  $ cat tmp-2.txt
  Global Hash Inputs
    Global Files                          = 1
    External Dependencies Hash            = ccab0b28617f1f56
    Global Cache Key                      = You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am.
    Global .env Files Considered          = 0
    Global Env Vars                       = SOME_ENV_VAR
    Global Env Vars Values                = 
    Inferred Global Env Vars Values       = 
    Global Passed Through Env Vars        = 
    Global Passed Through Env Vars Values = 

# Part 3 are Tasks to Run, and we have to validate each task separately
  $ cat tmp-3.txt | grep "my-app#build" -A 18
  my-app#build
    Task                           = build                                                                                                                                                                         
    Package                        = my-app                                                                                                                                                                        
    Hash                           = 0d1e6ee2c143211c                                                                                                                                                              
    Cached (Local)                 = false                                                                                                                                                                         
    Cached (Remote)                = false                                                                                                                                                                         
    Directory                      = apps/my-app                                                                                                                                                                   
    Command                        = echo 'building'                                                                                                                                                               
    Outputs                        = apple.json, banana.txt                                                                                                                                                        
    Log File                       = apps/my-app/.turbo/turbo-build.log                                                                                                                                            
    Dependencies                   =                                                                                                                                                                               
    Dependendents                  =                                                                                                                                                                               
    Inputs Files Considered        = 2                                                                                                                                                                             
    .env Files Considered          = 1                                                                                                                                                                             
    Env Vars                       =                                                                                                                                                                               
    Env Vars Values                =                                                                                                                                                                               
    Inferred Env Vars Values       =                                                                                                                                                                               
    Passed Through Env Vars        =                                                                                                                                                                               
    Passed Through Env Vars Values =                                                                                                                                                                               

  $ cat tmp-3.txt | grep "util#build" -A 18
  util#build
    Task                           = build                                                                                                                                                
    Package                        = util                                                                                                                                                 
    Hash                           = 76ab904c7ecb2d51                                                                                                                                     
    Cached (Local)                 = false                                                                                                                                                
    Cached (Remote)                = false                                                                                                                                                
    Directory                      = packages/util                                                                                                                                        
    Command                        = echo 'building'                                                                                                                                      
    Outputs                        =                                                                                                                                                      
    Log File                       = packages/util/.turbo/turbo-build.log                                                                                                                 
    Dependencies                   =                                                                                                                                                      
    Dependendents                  =                                                                                                                                                      
    Inputs Files Considered        = 1                                                                                                                                                    
    .env Files Considered          = 0                                                                                                                                                    
    Env Vars                       = NODE_ENV                                                                                                                                             
    Env Vars Values                =                                                                                                                                                      
    Inferred Env Vars Values       =                                                                                                                                                      
    Passed Through Env Vars        =                                                                                                                                                      
    Passed Through Env Vars Values =                                                                                                                                                      

# Run the task with NODE_ENV set and see it in summary. Use util package so it's just one package
  $ NODE_ENV=banana ${TURBO} run build --dry --filter=util | grep "Environment Variables"
  [1]
