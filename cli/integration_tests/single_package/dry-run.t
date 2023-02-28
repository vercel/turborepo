Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --dry --single-package
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"//#build":{"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false}}
  
  Tasks to Run
  build
    Task                    = build                                                                                                       
    Hash                    = 7bf32e1dedb04a5d                                                                                            
    Cached (Local)          = false                                                                                                       
    Cached (Remote)         = false                                                                                                       
    Command                 = echo 'building' > foo                                                                                       
    Outputs                 = foo                                                                                                         
    Log File                = .turbo/turbo-build.log                                                                                      
    Dependencies            =                                                                                                             
    Dependendents           =                                                                                                             
    Inputs Files Considered = 4                                                                                                           
    ResolvedTaskDefinition  = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 
    Framework               = <NO FRAMEWORK DETECTED>                                                                                     

  $ ${TURBO} run build --dry=json --single-package
  {
    "tasks": [
      {
        "task": "build",
        "hash": "7bf32e1dedb04a5d",
        "cacheState": {
          "local": false,
          "remote": false
        },
        "command": "echo 'building' \u003e foo",
        "outputs": [
          "foo"
        ],
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [
            "foo"
          ],
          "cache": true,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        },
        "expandedInputs": {
          ".gitignore": "6f23ff6842b5526da43ab38f4a5bf3b0158eeb42",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "185771929d92c3865ce06c863c07d357500d3364",
          "turbo.json": "2b9b71e8eca61cda6f4c14e07067feac9c1f9862"
        },
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e"
      }
    ]
  }
