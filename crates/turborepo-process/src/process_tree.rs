use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessTimes {
    pub(crate) creation: u64,
    pub(crate) exit: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessEntry {
    pub(crate) pid: u32,
    pub(crate) parent_pid: u32,
    pub(crate) times: Option<ProcessTimes>,
}

pub(crate) fn descendants(
    root_pid: u32,
    root_times: ProcessTimes,
    entries: &[ProcessEntry],
) -> Vec<u32> {
    let mut accepted = HashMap::from([(root_pid, root_times)]);
    let mut visited = HashSet::from([root_pid]);
    let mut current_generation = vec![root_pid];
    let mut descendants = Vec::new();

    while !current_generation.is_empty() {
        let mut next_generation = Vec::new();
        for entry in entries {
            if !current_generation.contains(&entry.parent_pid) || !visited.insert(entry.pid) {
                continue;
            }

            let Some(child_times) = entry.times else {
                continue;
            };
            let Some(parent_times) = accepted.get(&entry.parent_pid) else {
                continue;
            };

            if child_times.creation < parent_times.creation
                || parent_times
                    .exit
                    .is_some_and(|exit| child_times.creation > exit)
            {
                continue;
            }

            accepted.insert(entry.pid, child_times);
            descendants.push(entry.pid);
            next_generation.push(entry.pid);
        }
        current_generation = next_generation;
    }

    descendants
}

#[cfg(test)]
mod tests {
    use super::{ProcessEntry, ProcessTimes, descendants};

    fn times(creation: u64, exit: Option<u64>) -> Option<ProcessTimes> {
        Some(ProcessTimes { creation, exit })
    }

    #[test]
    fn rejects_dangling_old_parent() {
        let entries = [ProcessEntry {
            pid: 2,
            parent_pid: 1,
            times: times(50, None),
        }];

        assert!(
            descendants(
                1,
                ProcessTimes {
                    creation: 100,
                    exit: Some(200)
                },
                &entries
            )
            .is_empty()
        );
    }

    #[test]
    fn rejects_children_of_reused_root_pid() {
        let entries = [ProcessEntry {
            pid: 2,
            parent_pid: 1,
            times: times(201, None),
        }];

        assert!(
            descendants(
                1,
                ProcessTimes {
                    creation: 100,
                    exit: Some(200)
                },
                &entries
            )
            .is_empty()
        );
    }

    #[test]
    fn rejects_children_of_reused_intermediate_pid() {
        let entries = [
            ProcessEntry {
                pid: 2,
                parent_pid: 1,
                times: times(120, Some(150)),
            },
            ProcessEntry {
                pid: 3,
                parent_pid: 2,
                times: times(151, None),
            },
        ];

        assert_eq!(
            descendants(
                1,
                ProcessTimes {
                    creation: 100,
                    exit: Some(200)
                },
                &entries
            ),
            vec![2]
        );
    }

    #[test]
    fn accepts_valid_descendants_after_root_exit() {
        let entries = [
            ProcessEntry {
                pid: 2,
                parent_pid: 1,
                times: times(120, None),
            },
            ProcessEntry {
                pid: 3,
                parent_pid: 2,
                times: times(250, None),
            },
        ];

        assert_eq!(
            descendants(
                1,
                ProcessTimes {
                    creation: 100,
                    exit: Some(200)
                },
                &entries
            ),
            vec![2, 3]
        );
    }

    #[test]
    fn rejected_candidates_do_not_extend_traversal() {
        let entries = [
            ProcessEntry {
                pid: 2,
                parent_pid: 1,
                times: None,
            },
            ProcessEntry {
                pid: 3,
                parent_pid: 2,
                times: times(150, None),
            },
        ];

        assert!(
            descendants(
                1,
                ProcessTimes {
                    creation: 100,
                    exit: None
                },
                &entries
            )
            .is_empty()
        );
    }
}
