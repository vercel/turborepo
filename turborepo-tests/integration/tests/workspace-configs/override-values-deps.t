Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The override-values-task-with-deps configures dependsOn in the root turbo.json.
# The workspace does not have a turbo.json config. This test checks that both regular dependencies
# and Topological dependencies are retained from the root config.

# Run override-values-task-with-deps. In the root turbo.json it has two dependsOn values
# but in the workspace, we override to dependsOn: []. This test validates that only the
# top level task "override-values-task-with-deps" should run. None of the dependencies should run.
  $ ${TURBO} run override-values-task-with-deps --filter=override-values
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task-with-deps in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  override-values:override-values-task-with-deps: cache miss, executing 9b51bda96ea87896
  override-values:override-values-task-with-deps: 
  override-values:override-values-task-with-deps: > override-values-task-with-deps
  override-values:override-values-task-with-deps: > echo running-override-values-task-with-deps > out/foo.min.txt
  override-values:override-values-task-with-deps: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

# This is the same test as above, but with --dry and testing the resolvedTaskDefinition has the same value for dependsOn
  $ ${TURBO} run override-values-task-with-deps --filter=override-values --dry=json | jq '.tasks | map(select(.taskId == "override-values#override-values-task-with-deps")) | .[0].resolvedTaskDefinition'
  {
    "outputs": [],
    "cache": true,
    "dependsOn": [],
    "inputs": [],
    "outputLogs": "full",
    "persistent": false,
    "env": [],
    "passThroughEnv": null,
    "interactive": false
  }

# This task is similar, but `dependsOn` in the root turbo.json _only_ has a topological dependency
# This test was written to validate a common case of `build: dependsOn: [^build]`
  $ ${TURBO} run override-values-task-with-deps-2 --filter=override-values --dry=json | jq '.tasks | map(select(.taskId == "override-values#override-values-task-with-deps-2")) | .[0].resolvedTaskDefinition'
  {
    "outputs": [],
    "cache": true,
    "dependsOn": [],
    "inputs": [],
    "outputLogs": "full",
    "persistent": false,
    "env": [],
    "passThroughEnv": null,
    "interactive": false
  }
