//! Edge marks for task mode (`turborepo_trace_graph::taskmode`): listing-only
//! links and byte reads. Both are set on the traversal adjacency only; every
//! evaluation except that of build-task entries ignores them.
//!
//! **Listing-only links.** Generated-file linking makes a node that lists a
//! directory above a task's output depend on the task when the listing's
//! pattern can match a file the output produces ([`super::may_list_output`]). A
//! root-wide `**` or `*/*` can match anything, so one reader of such a listing
//! (a lint config that walks the repository, the whole-package hub of an
//! unknown command, the task-file discovery of every root task) was a consumer
//! of every task with outputs, and followed every input of each. A listing
//! *names* an output when its pattern says where the output is: its literal
//! leading directories lead into it (`dist/**/*.js`, `.next/types/**/*.ts`), or
//! one of its literal segments is the output's own name (`**/dist/*.js`). A
//! link made only by listings that do not name the output is marked
//! [`RefFlags::LISTED`]. So is every link of an unplaced write (`$BUILD_DIR/
//! pnpm-workspace.yaml`: a path with an unexpanded shell variable, which stands
//! for no file anyone can name).
//!
//! **Byte reads.** A `fs_read` or `fs_read_dir` edge from one file to another
//! file that no task regenerates is marked [`RefFlags::BYTES`]: the reader sees
//! the target's bytes, and those change only with the target itself. Only
//! tracked read calls count (`readFileSync`, `open`, `readdir`): an `asset`
//! edge and a located path ([`RefFlags::LOCATED`]: `new URL(x,
//! import.meta.url)`) say where a file is, not what is done with it (it may be
//! handed to a process or a bundler), and stay plain. An edge to a file that an
//! output of a task or task part covers stays plain: its bytes follow the
//! producer's inputs.

use turborepo_trace_graph::{Adjacency, EdgeAttr, NodeId, RefFlags, RefKind};
use turborepo_trace_tasks::glob::{expand_braces, is_glob, split_static_prefix};

use super::Graph;

/// Marker on the target of an extra edge while links are collected: a
/// listing-only link.
pub(super) const LISTED_LINK: u32 = 1 << 30;

/// The attribute of a listing-only link.
pub(super) fn listed_attr() -> EdgeAttr {
    EdgeAttr::new(
        RefKind::Import,
        turborepo_trace_graph::Phase::Runtime,
        RefFlags::LISTED,
    )
}

/// Whether an output path (repo-relative) is an unplaced write
/// (`turborepo_trace_tasks::spec::unplaced`): one of its components holds a
/// shell variable or a command substitution. Every link of such an output is
/// listing-only.
pub(super) fn unplaced(path: &str) -> bool {
    path.split('/').any(|c| {
        c.contains('`')
            || c.match_indices('$').any(|(i, _)| {
                c[i + 1..]
                    .chars()
                    .next()
                    .is_some_and(|n| n == '{' || n == '(' || n == '_' || n.is_ascii_alphanumeric())
            })
    })
}

/// Whether a listing with glob `pattern` of a directory names the output at
/// `rel` below it (`out_pattern`: the output's own glob below `rel`, if it has
/// one). See the module docs.
pub(super) fn names_output(pattern: &str, rel: &str, out_pattern: Option<&str>) -> bool {
    // Where the output is: `rel`, extended by the literal leading directories of
    // its own pattern (`<pkg>` with `dist/**` is at `<pkg>/dist`).
    let mut at = rel.trim_end_matches('/').to_owned();
    if let Some(op) = out_pattern {
        let (prefix, _) = split_static_prefix(op);
        if !prefix.is_empty() {
            if !at.is_empty() {
                at.push('/');
            }
            at.push_str(prefix);
        }
    }
    if at.is_empty() {
        return true;
    }
    let name = at.rsplit('/').next().unwrap_or(&at);
    expand_braces(pattern).iter().any(|alt| {
        let alt = alt.strip_prefix("./").unwrap_or(alt);
        let (prefix, _) = split_static_prefix(alt);
        let leads_in = !prefix.is_empty()
            && (prefix == at
                || prefix
                    .strip_prefix(at.as_str())
                    .is_some_and(|r| r.starts_with('/')));
        leads_in || alt.split('/').any(|seg| seg == name && !is_glob(seg))
    })
}

impl Graph {
    /// Marks byte reads on the traversal adjacency (see the module docs).
    /// `produced`: the nodes an output of a task covers (sorted): files a
    /// task regenerates.
    pub(super) fn mark_byte_reads(&mut self, produced: &[u32]) {
        let n = self.nodes.len();
        let (off, to, attrs) = self.fwd.parts();
        let regenerated = |t: usize| produced.binary_search(&(t as u32)).is_ok();
        let bit = (RefFlags::BYTES.bits() as u16) << 8;
        let mut marked: Vec<EdgeAttr> = attrs.to_vec();
        let mut any = false;
        for i in 0..n {
            if self.is_pseudo(i) {
                continue;
            }
            for k in off[i] as usize..off[i + 1] as usize {
                let t = to[k] as usize;
                if matches!(marked[k].kind(), RefKind::FsRead | RefKind::FsReadDir)
                    && !marked[k].flags().contains(RefFlags::LOCATED)
                    && !self.is_pseudo(t)
                    && !regenerated(t)
                {
                    marked[k] = EdgeAttr(marked[k].0 | bit);
                    any = true;
                }
            }
        }
        if any {
            let (off, to) = (off.to_vec(), to.to_vec());
            self.fwd = Adjacency::from_parts(off, to, marked);
        }
    }

    /// The number of traversal edges carrying `flag` (tests, statistics).
    pub fn flagged_edges(&self, flag: RefFlags) -> usize {
        (0..self.nodes.len() as u32)
            .map(|i| {
                self.fwd
                    .neighbors(NodeId(i))
                    .1
                    .iter()
                    .filter(|a| a.flags().contains(flag))
                    .count()
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::names_output;

    #[test]
    fn a_listing_names_an_output_by_its_literal_path_or_name() {
        // Literal leading directories that lead into the output.
        assert!(names_output("dist/**/*.js", "dist", None));
        assert!(names_output(".next/types/**/*.ts", ".next", None));
        assert!(!names_output("**/*.ts", "", Some(".next/types/**")));
        assert!(names_output(".next/types/**/*.ts", "", Some(".next/**")));
        assert!(names_output(
            "packages/ui/dist/*.js",
            "packages/ui",
            Some("dist/**")
        ));
        // A literal segment that is the output's own name.
        assert!(names_output("**/dist/*.js", "packages/ui/dist", None));
        assert!(names_output(
            "*/*/dist/index.{js,mjs}",
            "packages/ui",
            Some("dist/**")
        ));
        assert!(names_output(
            "src/generated/client.ts",
            "src/generated/client.ts",
            None
        ));
        // Wildcards alone: the listing would see the output, it does not name it.
        assert!(!names_output("**", "$BUILD_DIR/pnpm-workspace.yaml", None));
        assert!(!names_output("*/*", "packages/ui", Some("dist/**")));
        assert!(!names_output(
            "*/*/package.json",
            "packages/ui",
            Some("dist/**")
        ));
        assert!(!names_output("**/*.ts", "packages/ui/dist", None));
        assert!(!names_output("src/**/*.ts", "src/generated", None));
        assert!(!names_output("packages/ui/*", "packages/ui/dist", None));
        assert!(!names_output("**/__mocks__/**", "packages/ui/dist", None));
        // A literal directory above the output is not its name.
        assert!(!names_output("packages/*/src/**", "packages/ui/dist", None));
    }
}
