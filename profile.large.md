# CPU Profile

| Duration | Spans | Functions |
| -------- | ----- | --------- |
| 1.4s     | 16359 | 33        |

**Top 10:** `walk_glob` 50.6%, `compile_globs` 24.3%, `new` 19.7%, `git_status_repo_root` 19.7%, `queue_task` 14.8%, `get_package_file_hashes_from_inputs_and_index` 12.3%, `parse_lockfile` 7.6%, `finish` 7.0%, `parse_package_jsons` 6.6%, `calculate_file_hashes` 6.4%

## Hot Functions (Self Time)

| Self% |    Self | Total% |   Total | Function                                        | Location                                                                                                        |
| ----: | ------: | -----: | ------: | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| 50.6% | 714.5ms |  50.6% | 714.5ms | `walk_glob`                                     | `crates/turborepo-globwalk/src/lib.rs:601`                                                                      |
| 24.3% | 343.7ms |  26.0% | 367.6ms | `compile_globs`                                 | `crates/turborepo-globwalk/src/lib.rs:519`                                                                      |
| 19.7% | 278.3ms |  19.7% | 278.3ms | `new`                                           | `crates/turborepo-scm/src/repo_index.rs:20`                                                                     |
| 19.7% | 278.2ms |  19.7% | 278.2ms | `git_status_repo_root`                          | `crates/turborepo-scm/src/status.rs:56`                                                                         |
| 14.8% | 209.2ms |  21.1% | 298.5ms | `queue_task`                                    | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:205`                                                        |
| 12.3% | 173.4ms |  82.9% |    1.2s | `get_package_file_hashes_from_inputs_and_index` | `crates/turborepo-scm/src/package_deps.rs:244`                                                                  |
|  7.6% | 107.0ms |   7.6% | 107.0ms | `parse_lockfile`                                | `crates/turborepo-repository/src/package_manager/mod.rs:479`                                                    |
|  7.0% |  99.3ms |   7.0% |  99.3ms | `finish`                                        | `crates/turborepo-run-summary/src/tracker.rs:307`                                                               |
|  6.6% |  92.7ms |   6.6% |  93.7ms | `parse_package_jsons`                           | `crates/turborepo-repository/src/package_graph/builder.rs:289`                                                  |
|  6.4% |  90.9ms |   6.4% |  90.9ms | `calculate_file_hashes`                         | `crates/turborepo-task-hash/src/lib.rs:79`                                                                      |
|  6.3% |  89.3ms |   6.3% |  89.3ms | `calculate_task_hash`                           | `crates/turborepo-task-hash/src/lib.rs:290`                                                                     |
|  6.2% |  88.2ms |   6.2% |  88.2ms | `to_summary`                                    | `crates/turborepo-run-summary/src/tracker.rs:121`                                                               |
|  4.6% |  64.5ms |   4.6% |  64.5ms | `git_ls_tree_repo_root_sorted`                  | `crates/turborepo-scm/src/ls_tree.rs:41`                                                                        |
|  4.4% |  62.2ms |   4.4% |  62.2ms | `hash_objects`                                  | `crates/turborepo-scm/src/hash_object.rs:39`                                                                    |
|  3.6% |  50.3ms |   3.6% |  50.3ms | `connect_internal_dependencies`                 | `crates/turborepo-repository/src/package_graph/builder.rs:390`                                                  |
|  2.4% |  33.4ms |   2.4% |  33.4ms | `parse`                                         | `/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` |
|  2.0% |  28.9ms |   2.0% |  28.9ms | `populate_transitive_dependencies`              | `crates/turborepo-repository/src/package_graph/builder.rs:542`                                                  |
|  1.7% |  24.4ms |   1.9% |  26.9ms | `get_package_file_hashes_from_index`            | `crates/turborepo-scm/src/package_deps.rs:148`                                                                  |
|  1.7% |  24.0ms |   1.7% |  24.0ms | `preprocess_paths_and_globs`                    | `crates/turborepo-globwalk/src/lib.rs:71`                                                                       |
|  0.7% |  10.1ms |   0.7% |  10.1ms | `new`                                           | `crates/turborepo-scm/src/lib.rs:286`                                                                           |
|  0.5% |   7.0ms |  21.6% | 305.5ms | `visit`                                         | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:180`                                                        |
|  0.5% |   6.5ms |   0.5% |   6.5ms | `exists`                                        | `crates/turborepo-cache/src/fs.rs:126`                                                                          |

## Call Tree (Total Time)

| Total% |   Total | Self% |    Self | Function                                        | Location                                                                                                        |
| -----: | ------: | ----: | ------: | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
|  84.2% |    1.2s |  0.3% |   4.6ms | `get_package_file_hashes`                       | `crates/turborepo-scm/src/package_deps.rs:27`                                                                   |
|  82.9% |    1.2s | 12.3% | 173.4ms | `get_package_file_hashes_from_inputs_and_index` | `crates/turborepo-scm/src/package_deps.rs:244`                                                                  |
|  50.6% | 714.5ms | 50.6% | 714.5ms | `walk_glob`                                     | `crates/turborepo-globwalk/src/lib.rs:601`                                                                      |
|  26.0% | 367.6ms | 24.3% | 343.7ms | `compile_globs`                                 | `crates/turborepo-globwalk/src/lib.rs:519`                                                                      |
|  21.6% | 305.5ms |  0.5% |   7.0ms | `visit`                                         | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:180`                                                        |
|  21.1% | 298.5ms | 14.8% | 209.2ms | `queue_task`                                    | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:205`                                                        |
|  19.9% | 281.1ms |  0.0% |   451us | `build`                                         | `crates/turborepo-repository/src/package_graph/builder.rs:150`                                                  |
|  19.7% | 278.3ms | 19.7% | 278.3ms | `new`                                           | `crates/turborepo-scm/src/repo_index.rs:20`                                                                     |
|  19.7% | 278.2ms | 19.7% | 278.2ms | `git_status_repo_root`                          | `crates/turborepo-scm/src/status.rs:56`                                                                         |
|  11.2% | 158.0ms |  0.0% |    73us | `resolve_lockfile`                              | `crates/turborepo-repository/src/package_graph/builder.rs:467`                                                  |
|   7.6% | 107.6ms |  0.0% |    61us | `populate_lockfile`                             | `crates/turborepo-repository/src/package_graph/builder.rs:443`                                                  |
|   7.6% | 107.5ms |  0.0% |   538us | `read_lockfile`                                 | `crates/turborepo-repository/src/package_manager/mod.rs:458`                                                    |
|   7.6% | 107.0ms |  7.6% | 107.0ms | `parse_lockfile`                                | `crates/turborepo-repository/src/package_manager/mod.rs:479`                                                    |
|   7.0% |  99.3ms |  7.0% |  99.3ms | `finish`                                        | `crates/turborepo-run-summary/src/tracker.rs:307`                                                               |
|   6.6% |  93.7ms |  6.6% |  92.7ms | `parse_package_jsons`                           | `crates/turborepo-repository/src/package_graph/builder.rs:289`                                                  |
|   6.4% |  90.9ms |  6.4% |  90.9ms | `calculate_file_hashes`                         | `crates/turborepo-task-hash/src/lib.rs:79`                                                                      |
|   6.3% |  89.3ms |  6.3% |  89.3ms | `calculate_task_hash`                           | `crates/turborepo-task-hash/src/lib.rs:290`                                                                     |
|   6.2% |  88.2ms |  6.2% |  88.2ms | `to_summary`                                    | `crates/turborepo-run-summary/src/tracker.rs:121`                                                               |
|   4.6% |  64.5ms |  4.6% |  64.5ms | `git_ls_tree_repo_root_sorted`                  | `crates/turborepo-scm/src/ls_tree.rs:41`                                                                        |
|   4.4% |  62.2ms |  4.4% |  62.2ms | `hash_objects`                                  | `crates/turborepo-scm/src/hash_object.rs:39`                                                                    |
|   3.6% |  50.3ms |  3.6% |  50.3ms | `connect_internal_dependencies`                 | `crates/turborepo-repository/src/package_graph/builder.rs:390`                                                  |
|   2.4% |  33.4ms |  2.4% |  33.4ms | `parse`                                         | `/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` |
|   2.1% |  29.0ms |  0.0% |    65us | `build_inner`                                   | `crates/turborepo-repository/src/package_graph/builder.rs:561`                                                  |
|   2.0% |  28.9ms |  2.0% |  28.9ms | `populate_transitive_dependencies`              | `crates/turborepo-repository/src/package_graph/builder.rs:542`                                                  |
|   1.9% |  26.9ms |  1.7% |  24.4ms | `get_package_file_hashes_from_index`            | `crates/turborepo-scm/src/package_deps.rs:148`                                                                  |
|   1.7% |  24.0ms |  1.7% |  24.0ms | `preprocess_paths_and_globs`                    | `crates/turborepo-globwalk/src/lib.rs:71`                                                                       |
|   0.7% |  10.1ms |  0.7% |  10.1ms | `new`                                           | `crates/turborepo-scm/src/lib.rs:286`                                                                           |
|   0.5% |   6.5ms |  0.5% |   6.5ms | `exists`                                        | `crates/turborepo-cache/src/fs.rs:126`                                                                          |

## Function Details

### `walk_glob`

`crates/turborepo-globwalk/src/lib.rs:601` | Self: 50.6% (714.5ms) | Total: 50.6% (714.5ms) | Calls: 1009

**Called by:**

- `get_package_file_hashes_from_inputs_and_index` (992)

### `compile_globs`

`crates/turborepo-globwalk/src/lib.rs:519` | Self: 24.3% (343.7ms) | Total: 26.0% (367.6ms) | Calls: 994

**Called by:**

- `parse_package_jsons` (1)
- `get_package_file_hashes_from_inputs_and_index` (992)

**Calls:**

- `preprocess_paths_and_globs` (994)

### `new`

`crates/turborepo-scm/src/repo_index.rs:20` | Self: 19.7% (278.3ms) | Total: 19.7% (278.3ms) | Calls: 1

### `git_status_repo_root`

`crates/turborepo-scm/src/status.rs:56` | Self: 19.7% (278.2ms) | Total: 19.7% (278.2ms) | Calls: 1

### `queue_task`

`crates/turborepo-lib/src/task_graph/visitor/mod.rs:205` | Self: 14.8% (209.2ms) | Total: 21.1% (298.5ms) | Calls: 1690

**Called by:**

- `visit` (1690)

**Calls:**

- `calculate_task_hash` (1690)

### `get_package_file_hashes_from_inputs_and_index`

`crates/turborepo-scm/src/package_deps.rs:244` | Self: 12.3% (173.4ms) | Total: 82.9% (1.2s) | Calls: 992

**Called by:**

- `get_package_file_hashes` (992)

**Calls:**

- `walk_glob` (992)
- `hash_objects` (992)
- `get_package_file_hashes_from_index` (992)
- `compile_globs` (992)

### `parse_lockfile`

`crates/turborepo-repository/src/package_manager/mod.rs:479` | Self: 7.6% (107.0ms) | Total: 7.6% (107.0ms) | Calls: 1

**Called by:**

- `read_lockfile` (1)

### `finish`

`crates/turborepo-run-summary/src/tracker.rs:307` | Self: 7.0% (99.3ms) | Total: 7.0% (99.3ms) | Calls: 1

### `parse_package_jsons`

`crates/turborepo-repository/src/package_graph/builder.rs:289` | Self: 6.6% (92.7ms) | Total: 6.6% (93.7ms) | Calls: 1

**Called by:**

- `build` (1)

**Calls:**

- `compile_globs` (1)

### `calculate_file_hashes`

`crates/turborepo-task-hash/src/lib.rs:79` | Self: 6.4% (90.9ms) | Total: 6.4% (90.9ms) | Calls: 1

### `calculate_task_hash`

`crates/turborepo-task-hash/src/lib.rs:290` | Self: 6.3% (89.3ms) | Total: 6.3% (89.3ms) | Calls: 1690

**Called by:**

- `queue_task` (1690)

### `to_summary`

`crates/turborepo-run-summary/src/tracker.rs:121` | Self: 6.2% (88.2ms) | Total: 6.2% (88.2ms) | Calls: 1

### `git_ls_tree_repo_root_sorted`

`crates/turborepo-scm/src/ls_tree.rs:41` | Self: 4.6% (64.5ms) | Total: 4.6% (64.5ms) | Calls: 1

### `hash_objects`

`crates/turborepo-scm/src/hash_object.rs:39` | Self: 4.4% (62.2ms) | Total: 4.4% (62.2ms) | Calls: 2687

**Called by:**

- `get_package_file_hashes_from_index` (1688)
- `get_package_file_hashes_from_inputs_and_index` (992)

### `connect_internal_dependencies`

`crates/turborepo-repository/src/package_graph/builder.rs:390` | Self: 3.6% (50.3ms) | Total: 3.6% (50.3ms) | Calls: 1

**Called by:**

- `resolve_lockfile` (1)

### `parse`

`/Users/anthonyshew/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/biome_json_parser-0.5.7/src/lib.rs:32` | Self: 2.4% (33.4ms) | Total: 2.4% (33.4ms) | Calls: 1215

### `populate_transitive_dependencies`

`crates/turborepo-repository/src/package_graph/builder.rs:542` | Self: 2.0% (28.9ms) | Total: 2.0% (28.9ms) | Calls: 1

**Called by:**

- `build_inner` (1)

### `get_package_file_hashes_from_index`

`crates/turborepo-scm/src/package_deps.rs:148` | Self: 1.7% (24.4ms) | Total: 1.9% (26.9ms) | Calls: 1688

**Called by:**

- `get_package_file_hashes` (690)
- `get_package_file_hashes_from_inputs_and_index` (992)

**Calls:**

- `hash_objects` (1688)

### `preprocess_paths_and_globs`

`crates/turborepo-globwalk/src/lib.rs:71` | Self: 1.7% (24.0ms) | Total: 1.7% (24.0ms) | Calls: 1000

**Called by:**

- `compile_globs` (994)

### `new`

`crates/turborepo-scm/src/lib.rs:286` | Self: 0.7% (10.1ms) | Total: 0.7% (10.1ms) | Calls: 1
