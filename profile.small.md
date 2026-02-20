# CPU Profile

| Duration | Spans | Functions |
| -------- | ----- | --------- |
| 176.7ms  | 64    | 32        |

**Top 10:** `new` 50.2%, `git_status_repo_root` 50.1%, `git_ls_tree_repo_root_sorted` 26.8%, `new` 2.9%, `parse_package_jsons` 2.0%, `walk_glob` 1.5%, `parse_lockfile` 1.2%, `visit` 0.7%, `calculate_task_hash` 0.4%, `queue_task` 0.4%

## Hot Functions (Self Time)

| Self% |   Self | Total% |  Total | Function                       | Location                                                       |
| ----: | -----: | -----: | -----: | ------------------------------ | -------------------------------------------------------------- |
| 50.2% | 88.6ms |  50.2% | 88.6ms | `new`                          | `crates/turborepo-scm/src/repo_index.rs:20`                    |
| 50.1% | 88.5ms |  50.1% | 88.5ms | `git_status_repo_root`         | `crates/turborepo-scm/src/status.rs:56`                        |
| 26.8% | 47.3ms |  26.8% | 47.3ms | `git_ls_tree_repo_root_sorted` | `crates/turborepo-scm/src/ls_tree.rs:41`                       |
|  2.9% |  5.1ms |   2.9% |  5.1ms | `new`                          | `crates/turborepo-scm/src/lib.rs:286`                          |
|  2.0% |  3.5ms |   2.3% |  4.1ms | `parse_package_jsons`          | `crates/turborepo-repository/src/package_graph/builder.rs:289` |
|  1.5% |  2.7ms |   1.5% |  2.7ms | `walk_glob`                    | `crates/turborepo-globwalk/src/lib.rs:601`                     |
|  1.2% |  2.1ms |   1.2% |  2.1ms | `parse_lockfile`               | `crates/turborepo-repository/src/package_manager/mod.rs:479`   |
|  0.7% |  1.3ms |   1.5% |  2.6ms | `visit`                        | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:180`       |

## Call Tree (Total Time)

| Total% |  Total | Self% |   Self | Function                       | Location                                                       |
| -----: | -----: | ----: | -----: | ------------------------------ | -------------------------------------------------------------- |
|  50.2% | 88.6ms | 50.2% | 88.6ms | `new`                          | `crates/turborepo-scm/src/repo_index.rs:20`                    |
|  50.1% | 88.5ms | 50.1% | 88.5ms | `git_status_repo_root`         | `crates/turborepo-scm/src/status.rs:56`                        |
|  26.8% | 47.3ms | 26.8% | 47.3ms | `git_ls_tree_repo_root_sorted` | `crates/turborepo-scm/src/ls_tree.rs:41`                       |
|   4.3% |  7.6ms |  0.2% |  404us | `build`                        | `crates/turborepo-repository/src/package_graph/builder.rs:150` |
|   2.9% |  5.1ms |  2.9% |  5.1ms | `new`                          | `crates/turborepo-scm/src/lib.rs:286`                          |
|   2.3% |  4.1ms |  2.0% |  3.5ms | `parse_package_jsons`          | `crates/turborepo-repository/src/package_graph/builder.rs:289` |
|   1.6% |  2.7ms |  0.0% |    9us | `resolve_lockfile`             | `crates/turborepo-repository/src/package_graph/builder.rs:467` |
|   1.5% |  2.7ms |  1.5% |  2.7ms | `walk_glob`                    | `crates/turborepo-globwalk/src/lib.rs:601`                     |
|   1.5% |  2.6ms |  0.7% |  1.3ms | `visit`                        | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:180`       |
|   1.4% |  2.5ms |  0.0% |    3us | `populate_lockfile`            | `crates/turborepo-repository/src/package_graph/builder.rs:443` |
|   1.4% |  2.5ms |  0.2% |  404us | `read_lockfile`                | `crates/turborepo-repository/src/package_manager/mod.rs:458`   |
|   1.2% |  2.1ms |  1.2% |  2.1ms | `parse_lockfile`               | `crates/turborepo-repository/src/package_manager/mod.rs:479`   |
|   0.8% |  1.3ms |  0.4% |  657us | `queue_task`                   | `crates/turborepo-lib/src/task_graph/visitor/mod.rs:205`       |

## Function Details

### `new`

`crates/turborepo-scm/src/repo_index.rs:20` | Self: 50.2% (88.6ms) | Total: 50.2% (88.6ms) | Calls: 1

### `git_status_repo_root`

`crates/turborepo-scm/src/status.rs:56` | Self: 50.1% (88.5ms) | Total: 50.1% (88.5ms) | Calls: 1

### `git_ls_tree_repo_root_sorted`

`crates/turborepo-scm/src/ls_tree.rs:41` | Self: 26.8% (47.3ms) | Total: 26.8% (47.3ms) | Calls: 1

### `new`

`crates/turborepo-scm/src/lib.rs:286` | Self: 2.9% (5.1ms) | Total: 2.9% (5.1ms) | Calls: 1

### `parse_package_jsons`

`crates/turborepo-repository/src/package_graph/builder.rs:289` | Self: 2.0% (3.5ms) | Total: 2.3% (4.1ms) | Calls: 1

**Called by:**

- `build` (1)

**Calls:**

- `compile_globs` (1)

### `walk_glob`

`crates/turborepo-globwalk/src/lib.rs:601` | Self: 1.5% (2.7ms) | Total: 1.5% (2.7ms) | Calls: 2

### `parse_lockfile`

`crates/turborepo-repository/src/package_manager/mod.rs:479` | Self: 1.2% (2.1ms) | Total: 1.2% (2.1ms) | Calls: 1

**Called by:**

- `read_lockfile` (1)

### `visit`

`crates/turborepo-lib/src/task_graph/visitor/mod.rs:180` | Self: 0.7% (1.3ms) | Total: 1.5% (2.6ms) | Calls: 1

**Calls:**

- `queue_task` (5)
