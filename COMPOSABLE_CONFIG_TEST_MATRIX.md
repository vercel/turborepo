# Test cases for composable config

## General

- [ ] Missing task definition in root, can add task from override.

- [ ] add-keys

  - [x] dependsOn
  - [x] inputs
  - [x] outputs
  - [x] env
  - [ ] cache
  - [x] outputMode

- [ ] omit keys

  - [ ] dependsOn
  - [ ] inputs
  - [ ] outputs
  - [ ] env
  - [ ] cache
  - [ ] outputMode

- `dependsOn`
  exercise by: run task, expect dependent task to only run when appropriate

  - [x] add-key: No `dependsOn` in root, add in workspace
  - [x] omit-key: Add `dependsOn` in root, omit key in workspace
  - [x] override-value: Add `dependsOn` in root, override in workspace
  - [ ] no-config: Add `dependsOn` in root, have no workspace turbo.json

- `outputs`
  exercise by: run task by writing files to multiple places, expect correct folder is cached

  - [x] add-key: No `outputs` in root, add in workspace
  - [x] omit-key: Add `outputs` in root, omit key in workspace
  - [x] override-value: Add `outputs` in root, override to something else in workspace
  - [ ] no-config: Add `outputs` in root, have no workspace turbo.json

- `env`
  exercise by: run task, set env var, run again, expect has is different

  - [ ] add-keyNo `env` in root, add in workspace
  - [ ] omit-key: Add `env` in root, omit key in workspace
  - [ ] override-value: Add `env` in root, override to `[]` in workspace
  - [ ] no-config: Add `env` in root, have no workspace turbo.json

- `inputs`
  exercise by: run task, change input, run again and expect cache miss in the right places

  - [ ] add-key: No `inputs` in root, add in workspace
  - [ ] omit-key: Add `inputs` in root, omit key in workspace
  - [ ] override-value: Add `inputs` in root, override to `[]` in workspace
  - [ ] no-config: Add `inputs` in root, have no workspace turbo.json

- `cache`
  exercise by: run task, expect overriden workspace not to have a cache

  - [ ] add-key: No `cache` in root, add `false` in workspace
  - [ ] override-value: Add `cache:false` in root, override to `true` in workspace

- `outputMode`
  exercise by: run task, expect correct log output

  - [ ] add-key: No `outputMode` in root, add in workspace
  - [ ] override-value: Add `outputMode` to root, override in workspace
  - [ ] omit-key: Add `outputMode` to root, omit in workspace with turbo.json
  - [ ] no-config: Add `outputMode` to root, no turbo.json in workspace

- `persistent`

  exercise by: run task with persistent dependency and expect an error in the right place

  - [ ] Add `persistent:true` to root, override to `false` in workspace
  - [ ] Add `persistent:true` to root, omit in workspace with turbo.json
  - [ ] Add `persistent.true` to root, no override in workspace
  - [ ] No `persistent` flag in workspace, add `true` in workspace
