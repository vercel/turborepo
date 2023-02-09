Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

  $ git ls-tree -r HEAD
  100644 blob 6f23ff6842b5526da43ab38f4a5bf3b0158eeb42\t.gitignore (esc)
  100644 blob 1c117cce37347befafe3a9cba1b8a609b3600021\tpackage-lock.json (esc)
  100644 blob 185771929d92c3865ce06c863c07d357500d3364\tpackage.json (esc)
  100644 blob 2b9b71e8eca61cda6f4c14e07067feac9c1f9862\tturbo.json (esc)

Check
  $ ${TURBO} run build --dry --single-package
  
  Tasks to Run
  build
    Task                   = build                                                                                                       
    Hash                   = 7bf32e1dedb04a5d                                                                                            
    Cached (Local)         = false                                                                                                       
    Cached (Remote)        = false                                                                                                       
    Command                = echo 'building' > foo                                                                                       
    Outputs                = foo                                                                                                         
    Log File               = .turbo/turbo-build.log                                                                                      
    Dependencies           =                                                                                                             
    Dependendents          =                                                                                                             
    ResolvedTaskDefinition = {"outputs":["foo"],"cache":true,"dependsOn":[],"inputs":[],"outputMode":"full","env":[],"persistent":false} 

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
        }
      }
    ]
  }

  $ ${TURBO} run build --dry --single-package -vv > logs 2>&1 
  $ cat logs | grep "taskHashInputs"
  [-0-9:.TWZ+]+ \[DEBUG] turbo: taskhash.taskHashInputs{packageDir:"", hashOfFiles:"c3513037e56ad478", externalDepsHash:"", task:"build", outputs:fs.TaskOutputs{Inclusions:\[]string(nil), Exclusions:\[]string(nil)}, passThruArgs:\[]string{}, hashableEnvPairs:[]string{}, globalHash:"0d2c84a1b8ca6878", taskDependencyHashes:\[]string{}} (re)
