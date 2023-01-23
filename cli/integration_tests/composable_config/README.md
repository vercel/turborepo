# Test cases for composable config

- `dependsOn`
  exercise by: run task, expect dependent task to only run when appropriate

  - [x] Add `dependsOn` in root turbo.json, override to `[]` in workspace
  - [ ] Add `dependsOn` in root turbo.json, omit key in workspace
  - [x] Add `dependsOn` in root turbo.json, have no workspace turbo.json
  - [ ] No `dependsOn` in root turbo.json, add in workspace

- `env`
  exercise by: run task, set env var, run again, expect has is different

  - [ ] Add `env` in root turbo.json, override to `[]` in workspace
  - [ ] Add `env` in root turbo.json, omit key in workspace
  - [ ] Add `env` in root turbo.json, have no workspace turbo.json
  - [ ] No `env` in root turbo.json, add in workspace

- `outputs`
  exercise by: run task by writing files to multiple places, expect correct folder is cached

  - [x] Add `outputs` in root turbo.json, override to something else in workspace
  - [ ] Add `outputs` in root turbo.json, omit key in workspace
  - [x] Add `outputs` in root turbo.json, have no workspace turbo.json
  - [ ] No `outputs` in root turbo.json, add in workspace

- `inputs`
  exercise by: run task, change input, run again and expect cache miss in the right places

  - [ ] Add `inputs` in root turbo.json, override to `[]` in workspace
  - [ ] Add `inputs` in root turbo.json, omit key in workspace
  - [ ] Add `inputs` in root turbo.json, have no workspace turbo.json
  - [ ] No `inputs` in root turbo.json, add in workspace

- `cache`
  exercise by: run task, expect overriden workspace not to have a cache

  - [ ] Add `cache:false` in root turbo.json, override to `true` in workspace
  - [ ] No `cache` in root turbo.json, and no override in workspace
  - [ ] No `cache` in root turbo.json, add `false` in workspace

- `outputMode`
  exercise by: run task, expect correct log output

  - [ ] Add `outputMode` to root turbo.json, override in workspace
  - [ ] Add `outputMode` to root turbo.json, omit in workspace with turbo.json
  - [ ] Add `outputMode` to root turbo.json, no turbo.json in workspace
  - [ ] No `outputMode` in root turbo.json, add in workspace

- `persistent`

  exercise by: run task with persistent dependency and expect an error in the right place

  - [ ] Add `persistent:true` to root turbo.json, override to `false` in workspace
  - [ ] Add `persistent:true` to root turbo.json, omit in workspace with turbo.json
  - [ ] Add `persistent.true` to root turbo.json, no override in workspace
  - [ ] No `persistent` flag in workspace, add `true` in workspace
