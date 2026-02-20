# CPU Profile

| Duration | Spans | Functions |
| -------- | ----- | --------- |
| 804.1ms  | 2017  | 33        |

**Top 10:** `new` 72.2%, `git_status_repo_root` 72.1%, `git_ls_tree_repo_root_sorted` 43.4%, `walk_glob` 14.0%, `parse_lockfile` 9.6%, `compile_globs` 4.8%, `calculate_task_hash` 4.2%, `exists` 3.2%, `parse_package_jsons` 3.1%, `queue_task` 2.4%

## Hot Functions (Self Time)

| Self% |    Self | Total% |   Total | Function                                        | Location                                                                                                        |
| ----: | ------: | -----: | ------: | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| 72.2% | 580.2ms |  72.2% | 580.2ms | `new`                                           | `crates/turborepo-scm/src/repo_index.rs:20`                                                                     |
| 72.1% | 580.1ms |  72.1% | 580.1ms | `git_status_repo_root`                          | `crates/turborepo-scm/src/status.rs:56`                                                                         |
| 43.4% | 348.9ms |  43.4% | 348.9ms | `git_ls_tree_repo_root_sorted`                  | `crates/turborepo-scm/src/ls_tree.rs:41`                                                                        |
| 14.0% | 112.5ms |  14.0% | 112.5ms | `walk_glob`                                     | `crates/turborepo-globwalk/src/lib.rs:601`                                                                      |
|  9.6% |  77.1ms |   9.6% |  77.1ms | `parse_lockfile`                                | `crates/turborepo-repository/src/package_manager/mod.rs:479`                                                    |
|  4.8% |  38.6ms |   5.2% |  41.9ms | `compile_globs`                                 | `crates/turborepo-globwalk/src/lib.rs:519`                                                                      |
|  4.2% |  33.4ms |   4.2% |  33.4ms | `calculate_task_hash`                           | `crates/turborepo-task-hash/src/lib.rs:290`                                                                     |
|  3.2% |  25.7ms |   3.2% |  25.7ms | `exists`                                        | `crates/turborepo-cache/src/fs.rs:126`                                                                          |
|  3.1% |  24.7ms |   3.2% |  25.4ms | `parse_package_jsons`                           | `crates/turborepo-repository/src/package_graph/builder.rs:289`                                                  |
|  2.4% |  19.4ms |   6.6% |  52.8ms | `queue_task`                                    | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:205`                                                        |
|  2.4% |  19.0ms |   2.4% |  19.0ms | `to_summary`                                    | `crates/turborepo-run-summary/src/tracker.rs:121`                                                               |
|  1.7% |  13.3ms |   1.7% |  13.5ms | `get_package_file_hashes_from_index`            | `crates/turborepo-scm/src/package_deps.rs:148`                                                                  |
|  1.6% |  13.0ms |   1.6% |  13.0ms | `finish`                                        | `crates/turborepo-run-summary/src/tracker.rs:307`                                                               |
|  1.6% |  12.9ms |   1.6% |  12.9ms | `calculate_file_hashes`                         | `crates/turborepo-task-hash/src/lib.rs:79`                                                                      |
|  1.4% |  11.4ms |   1.4% |  11.4ms | `hash_objects`                                  | `crates/turborepo-scm/src/hash_object.rs:39`                                                                    |
|  1.0% |   8.3ms |   1.0% |   8.3ms | `parse`                                         | `/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` |
|  0.9% |   7.4ms |   0.9% |   7.4ms | `populate_transitive_dependencies`              | `crates/turborepo-repository/src/package_graph/builder.rs:542`                                                  |
|  0.7% |   5.8ms |  17.3% | 138.8ms | `get_package_file_hashes_from_inputs_and_index` | `crates/turborepo-scm/src/package_deps.rs:244`                                                                  |
|  0.6% |   4.7ms |   0.6% |   4.7ms | `new`                                           | `crates/turborepo-scm/src/lib.rs:286`                                                                           |
|  0.5% |   4.2ms |   0.5% |   4.2ms | `connect_internal_dependencies`                 | `crates/turborepo-repository/src/package_graph/builder.rs:390`                                                  |
|  0.4% |   3.3ms |   0.4% |   3.3ms | `preprocess_paths_and_globs`                    | `crates/turborepo-globwalk/src/lib.rs:71`                                                                       |

## Call Tree (Total Time)

| Total% |   Total | Self% |    Self | Function                                        | Location                                                                                                        |
| -----: | ------: | ----: | ------: | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
|  72.2% | 580.2ms | 72.2% | 580.2ms | `new`                                           | `crates/turborepo-scm/src/repo_index.rs:20`                                                                     |
|  72.1% | 580.1ms | 72.1% | 580.1ms | `git_status_repo_root`                          | `crates/turborepo-scm/src/status.rs:56`                                                                         |
|  43.4% | 348.9ms | 43.4% | 348.9ms | `git_ls_tree_repo_root_sorted`                  | `crates/turborepo-scm/src/ls_tree.rs:41`                                                                        |
|  18.3% | 147.1ms |  0.0% |   374us | `get_package_file_hashes`                       | `crates/turborepo-scm/src/package_deps.rs:27`                                                                   |
|  17.3% | 138.8ms |  0.7% |   5.8ms | `get_package_file_hashes_from_inputs_and_index` | `crates/turborepo-scm/src/package_deps.rs:244`                                                                  |
|  14.4% | 115.6ms |  0.1% |   419us | `build`                                         | `crates/turborepo-repository/src/package_graph/builder.rs:150`                                                  |
|  14.0% | 112.5ms | 14.0% | 112.5ms | `walk_glob`                                     | `crates/turborepo-globwalk/src/lib.rs:601`                                                                      |
|  10.2% |  82.3ms |  0.0% |    14us | `resolve_lockfile`                              | `crates/turborepo-repository/src/package_graph/builder.rs:467`                                                  |
|   9.7% |  78.1ms |  0.0% |    10us | `populate_lockfile`                             | `crates/turborepo-repository/src/package_graph/builder.rs:443`                                                  |
|   9.7% |  78.0ms |  0.1% |   968us | `read_lockfile`                                 | `crates/turborepo-repository/src/package_manager/mod.rs:458`                                                    |
|   9.6% |  77.1ms |  9.6% |  77.1ms | `parse_lockfile`                                | `crates/turborepo-repository/src/package_manager/mod.rs:479`                                                    |
|   6.7% |  53.7ms |  0.1% |   953us | `visit`                                         | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:180`                                                        |
|   6.6% |  52.8ms |  2.4% |  19.4ms | `queue_task`                                    | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:205`                                                        |
|   5.2% |  41.9ms |  4.8% |  38.6ms | `compile_globs`                                 | `crates/turborepo-globwalk/src/lib.rs:519`                                                                      |
|   4.2% |  33.4ms |  4.2% |  33.4ms | `calculate_task_hash`                           | `crates/turborepo-task-hash/src/lib.rs:290`                                                                     |
|   3.2% |  25.7ms |  3.2% |  25.7ms | `exists`                                        | `crates/turborepo-cache/src/fs.rs:126`                                                                          |
|   3.2% |  25.4ms |  3.1% |  24.7ms | `parse_package_jsons`                           | `crates/turborepo-repository/src/package_graph/builder.rs:289`                                                  |
|   2.4% |  19.0ms |  2.4% |  19.0ms | `to_summary`                                    | `crates/turborepo-run-summary/src/tracker.rs:121`                                                               |
|   1.7% |  13.5ms |  1.7% |  13.3ms | `get_package_file_hashes_from_index`            | `crates/turborepo-scm/src/package_deps.rs:148`                                                                  |
|   1.6% |  13.0ms |  1.6% |  13.0ms | `finish`                                        | `crates/turborepo-run-summary/src/tracker.rs:307`                                                               |
|   1.6% |  12.9ms |  1.6% |  12.9ms | `calculate_file_hashes`                         | `crates/turborepo-task-hash/src/lib.rs:79`                                                                      |
|   1.4% |  11.4ms |  1.4% |  11.4ms | `hash_objects`                                  | `crates/turborepo-scm/src/hash_object.rs:39`                                                                    |
|   1.0% |   8.3ms |  1.0% |   8.3ms | `parse`                                         | `/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` |
|   0.9% |   7.5ms |  0.0% |    15us | `build_inner`                                   | `crates/turborepo-repository/src/package_graph/builder.rs:561`                                                  |
|   0.9% |   7.4ms |  0.9% |   7.4ms | `populate_transitive_dependencies`              | `crates/turborepo-repository/src/package_graph/builder.rs:542`                                                  |
|   0.6% |   4.7ms |  0.6% |   4.7ms | `new`                                           | `crates/turborepo-scm/src/lib.rs:286`                                                                           |
|   0.5% |   4.2ms |  0.5% |   4.2ms | `connect_internal_dependencies`                 | `crates/turborepo-repository/src/package_graph/builder.rs:390`                                                  |
|   0.4% |   3.3ms |  0.4% |   3.3ms | `preprocess_paths_and_globs`                    | `crates/turborepo-globwalk/src/lib.rs:71`                                                                       |

## Function Details

### `new`

`crates/turborepo-scm/src/repo_index.rs:20` | Self: 72.2% (580.2ms) | Total: 72.2% (580.2ms) | Calls: 1

### `git_status_repo_root`

`crates/turborepo-scm/src/status.rs:56` | Self: 72.1% (580.1ms) | Total: 72.1% (580.1ms) | Calls: 1

### `git_ls_tree_repo_root_sorted`

`crates/turborepo-scm/src/ls_tree.rs:41` | Self: 43.4% (348.9ms) | Total: 43.4% (348.9ms) | Calls: 1

### `walk_glob`

`crates/turborepo-globwalk/src/lib.rs:601` | Self: 14.0% (112.5ms) | Total: 14.0% (112.5ms) | Calls: 127

**Called by:**

- `get_package_file_hashes_from_inputs_and_index` (119)

### `parse_lockfile`

`crates/turborepo-repository/src/package_manager/mod.rs:479` | Self: 9.6% (77.1ms) | Total: 9.6% (77.1ms) | Calls: 1

**Called by:**

- `read_lockfile` (1)

### `compile_globs`

`crates/turborepo-globwalk/src/lib.rs:519` | Self: 4.8% (38.6ms) | Total: 5.2% (41.9ms) | Calls: 121

**Called by:**

- `parse_package_jsons` (1)
- `get_package_file_hashes_from_inputs_and_index` (119)

**Calls:**

- `preprocess_paths_and_globs` (121)

### `calculate_task_hash`

`crates/turborepo-task-hash/src/lib.rs:290` | Self: 4.2% (33.4ms) | Total: 4.2% (33.4ms) | Calls: 203

**Called by:**

- `queue_task` (203)

### `exists`

`crates/turborepo-cache/src/fs.rs:126` | Self: 3.2% (25.7ms) | Total: 3.2% (25.7ms) | Calls: 203

### `parse_package_jsons`

`crates/turborepo-repository/src/package_graph/builder.rs:289` | Self: 3.1% (24.7ms) | Total: 3.2% (25.4ms) | Calls: 1

**Called by:**

- `build` (1)

**Calls:**

- `compile_globs` (1)

### `queue_task`

`crates/turborepo-lib/src/task_graph/visitor/mod.rs:205` | Self: 2.4% (19.4ms) | Total: 6.6% (52.8ms) | Calls: 203

**Called by:**

- `visit` (203)

**Calls:**

- `calculate_task_hash` (203)

### `to_summary`

`crates/turborepo-run-summary/src/tracker.rs:121` | Self: 2.4% (19.0ms) | Total: 2.4% (19.0ms) | Calls: 1

### `get_package_file_hashes_from_index`

`crates/turborepo-scm/src/package_deps.rs:148` | Self: 1.7% (13.3ms) | Total: 1.7% (13.5ms) | Calls: 202

**Called by:**

- `get_package_file_hashes` (81)
- `get_package_file_hashes_from_inputs_and_index` (119)

**Calls:**

- `hash_objects` (202)

### `finish`

`crates/turborepo-run-summary/src/tracker.rs:307` | Self: 1.6% (13.0ms) | Total: 1.6% (13.0ms) | Calls: 1

### `calculate_file_hashes`

`crates/turborepo-task-hash/src/lib.rs:79` | Self: 1.6% (12.9ms) | Total: 1.6% (12.9ms) | Calls: 1

### `hash_objects`

`crates/turborepo-scm/src/hash_object.rs:39` | Self: 1.4% (11.4ms) | Total: 1.4% (11.4ms) | Calls: 324

**Called by:**

- `get_package_file_hashes_from_index` (202)
- `get_package_file_hashes_from_inputs_and_index` (119)

### `parse`

`/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` | Self: 1.0% (8.3ms) | Total: 1.0% (8.3ms) | Calls: 170

### `populate_transitive_dependencies`

`crates/turborepo-repository/src/package_graph/builder.rs:542` | Self: 0.9% (7.4ms) | Total: 0.9% (7.4ms) | Calls: 1

**Called by:**

- `build_inner` (1)

### `get_package_file_hashes_from_inputs_and_index`

`crates/turborepo-scm/src/package_deps.rs:244` | Self: 0.7% (5.8ms) | Total: 17.3% (138.8ms) | Calls: 119

**Called by:**

- `get_package_file_hashes` (119)

**Calls:**

- `get_package_file_hashes_from_index` (119)
- `compile_globs` (119)
- `hash_objects` (119)
- `walk_glob` (119)

### `new`

`crates/turborepo-scm/src/lib.rs:286` | Self: 0.6% (4.7ms) | Total: 0.6% (4.7ms) | Calls: 1

### `connect_internal_dependencies`

`crates/turborepo-repository/src/package_graph/builder.rs:390` | Self: 0.5% (4.2ms) | Total: 0.5% (4.2ms) | Calls: 1

**Called by:**

- `resolve_lockfile` (1)
