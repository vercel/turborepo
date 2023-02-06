# Test cases for composable config

## General

- [x] Missing task definition in root, can add task from override.
- [x] add-keys
  - [x] dependsOn
  - [x] inputs
  - [x] outputs
  - [x] env
  - [x] outputMode
- [x] omit keys
  - [x] dependsOn
  - [x] inputs
  - [x] outputs
  - [x] env
  - [x] outputMode
- [x] override-values
  - [x] dependsOn
  - [x] inputs
  - [x] outputs
  - [x] env
  - [x] outputMode
- [x] missing-workspace-config
  - [x] dependsOn
  - [x] inputs
  - [x] outputs
  - [x] env
  - [x] outputMode
- [ ]`cache`
  - [ ] Task that has cache:false in root can be overriden to cache:true in workspace
  - [ ] Task that has cache:true in root can be overriden to cache:false in workspace
  - [ ] Task that no cache config root can set cache:false in workspace
  - [ ] Task that has cache:false in root still works if workspace has no turbo.json
- `persistent`
  - [ ] Add `persistent:true` to root, override to `false` in workspace
  - [ ] Add `persistent:true` to root, omit in workspace with turbo.json
  - [ ] Add `persistent.true` to root, no override in workspace
  - [ ] No `persistent` flag in workspace, add `true` in workspace
