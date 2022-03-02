package run

import (
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/logger"
)

func RunCmd(ch *cmdutil.Helper) *cobra.Command {
	var opts struct {
		scope          string
		cacheDir       string
		concurrency    int
		shouldContinue bool
		force          bool
		graph          bool
		globalDeps     []string
		since          string
		ignore         string
		parallel       bool
		includeDeps    bool
		noDeps         bool
		noCache        bool
		cwd            string
		stream         bool
		only           bool
	}

	cmd := &cobra.Command{
		Use:   "run",
		Short: "Run tasks across projects in your monorepo",
		Long: `Run tasks across projects in your monorepo.

By default, turbo executes tasks in topological order (i.e.
dependencies first) and then caches the results. Re-running commands for
tasks already in the cache will skip re-execution and immediately move
artifacts from the cache into the correct output folders (as if the task
occurred again).
`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			// Force streaming output in CI/CD non-interactive mode
			if !logger.IsTTY || logger.IsCI {
				opts.stream = true
			}

			// We can only set this cache folder after we know actual cwd
			opts.cacheDir = filepath.Join(opts.cwd, opts.cacheDir)

			return nil
		},
	}

	path, err := os.Getwd()
	if err != nil {
		return nil
	}

	cmd.Flags().StringVar(&opts.scope, "scope", "", "package(s) to act as entry points for task execution, supports globs")
	cmd.Flags().StringVar(&opts.cacheDir, "cache-dir", "node_modules/.cache/turbo", "Specify local filesystem cache directory")
	cmd.Flags().IntVar(&opts.concurrency, "concurrency", 10, "concurrency of task execution")
	cmd.Flags().BoolVar(&opts.shouldContinue, "continue", false, "continue execution even if a task exits with an error or non-zero exit code")
	cmd.Flags().BoolVarP(&opts.force, "force", "f", false, "ignore the existing cache")
	cmd.Flags().BoolVarP(&opts.graph, "graph", "g", false, "generate a Dot graph of the task execution")
	cmd.Flags().StringArrayVar(&opts.globalDeps, "global-deps", []string{}, "glob of global filesystem dependencies to be hashed")
	cmd.Flags().StringVar(&opts.since, "since", "", "limit/set scope to changed packages since a mergebase")
	cmd.Flags().StringVar(&opts.ignore, "ignore", "", "files to ignore when calculating changed files, supports globs")
	cmd.Flags().BoolVarP(&opts.parallel, "parallel", "p", false, "execute all tasks in parallel")
	cmd.Flags().BoolVar(&opts.includeDeps, "include-deps", false, "include the dependencies of tasks in execution")
	cmd.Flags().BoolVar(&opts.noDeps, "no-deps", false, "exclude dependent task consumers from execution")
	cmd.Flags().BoolVar(&opts.noCache, "no-cache", false, "avoid saving task results to the cache")
	cmd.Flags().StringVar(&opts.cwd, "cwd", path, "directory to execute command in")
	cmd.Flags().BoolVar(&opts.stream, "stream", false, "stream???")
	cmd.Flags().BoolVar(&opts.only, "only", true, "only???")

	cmd.Flags().MarkHidden("stream")
	cmd.Flags().MarkHidden("only")

	return cmd
}
