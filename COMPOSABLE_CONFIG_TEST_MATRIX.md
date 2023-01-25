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

- [ ] override-values

  - [ ] dependsOn
  - [ ] inputs
  - [ ] outputs
  - [ ] env
  - [ ] cache
  - [ ] outputMode

- [ ] missing-workspace-config

  - [ ] dependsOn
  - [ ] inputs
  - [ ] outputs
  - [ ] env
  - [ ] cache
  - [ ] outputMode

- `cache`

  - [ ] Task that has cache:false in root can be overriden to cache:true in workspace
  - [ ] Task that has cache:true in root can be overriden to cache:false in workspace
  - [ ] Task that no cache config root can set cache:false in workspace
  - [ ] Task that has cache:false in root still works if workspace has no turbo.json

- `persistent`

  exercise by: run task with persistent dependency and expect an error in the right place

  - [ ] Add `persistent:true` to root, override to `false` in workspace
  - [ ] Add `persistent:true` to root, omit in workspace with turbo.json
  - [ ] Add `persistent.true` to root, no override in workspace
  - [ ] No `persistent` flag in workspace, add `true` in workspace
