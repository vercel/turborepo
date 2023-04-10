Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package_no_config

Check
  $ ${TURBO} run build --dry --single-package
  
  Global Hash Inputs
    Global Files               = 2
    External Dependencies Hash = 
    Global Cache Key           = Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo
    Root pipeline              = {"//#build":{"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false}}
  
  Tasks to Run
  build
    Task                             = build                                                                                                   
    Hash                             = c7223f212c321d3b                                                                                        
    Cached (Local)                   = false                                                                                                   
    Cached (Remote)                  = false                                                                                                   
    Command                          = echo 'building'                                                                                         
    Outputs                          =                                                                                                         
    Log File                         = .turbo/turbo-build.log                                                                                  
    Dependencies                     =                                                                                                         
    Dependendents                    =                                                                                                         
    Inputs Files Considered          = 3                                                                                                       
    Configured Environment Variables =                                                                                                         
    Inferred Environment Variables   =                                                                                                         
    Global Environment Variables     = VERCEL_ANALYTICS_ID=                                                                                    
    ResolvedTaskDefinition           = {"outputs":[],"cache":false,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 
    Framework                        = <NO FRAMEWORK DETECTED>                                                                                 

  $ ${TURBO} run build --dry=json --single-package
  {
    "id": "[a-zA-Z0-9]+", (re)
    "version": "0",
    "turboVersion": "[a-z0-9\.-]+", (re)
    "globalCacheInputs": {
      "rootKey": "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo",
      "files": {
        "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
        "package.json": "581fe2b8dcba5b03cbe51d78a973143eb6d33e3a"
      },
      "hashOfExternalDependencies": "",
      "rootPipeline": {
        "//#build": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        }
      }
    },
    "tasks": [
      {
        "task": "build",
        "hash": "c7223f212c321d3b",
        "inputs": {
          ".gitignore": "38548b0538f2fc563d6bacf70dd42798c6fd9a35",
          "package-lock.json": "8db0df575e6509336a6719094b63eb23d2c649c1",
          "package.json": "581fe2b8dcba5b03cbe51d78a973143eb6d33e3a"
        },
        "hashOfExternalDependencies": "",
        "cache": {
          "local": false,
          "remote": false
        },
        "command": "echo 'building'",
        "cliArguments": [],
        "outputs": null,
        "excludedOutputs": null,
        "logFile": ".turbo/turbo-build.log",
        "dependencies": [],
        "dependents": [],
        "resolvedTaskDefinition": {
          "outputs": [],
          "cache": false,
          "dependsOn": [],
          "inputs": [],
          "outputMode": "full",
          "env": [],
          "persistent": false
        },
        "expandedOutputs": [],
        "framework": "\u003cNO FRAMEWORK DETECTED\u003e",
        "environmentVariables": {
          "configured": [],
          "inferred": [],
          "global": [
            "VERCEL_ANALYTICS_ID="
          ]
        }
      }
    ]
  }
