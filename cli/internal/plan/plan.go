package plan

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/scm"
	"github.com/vercel/turborepo/cli/internal/scope"
	"github.com/vercel/turborepo/cli/internal/ui"
)

type PlanCommand struct {
	config *config.Config
	ui     cli.Ui
}

func NewCmd(config *config.Config, UI cli.Ui) *PlanCommand {
	return &PlanCommand{
		config: config,
		ui:     UI,
	}
}

func (pc *PlanCommand) Help() string {
	cmd := pc.getCmd()
	return cmd.UsageString()
}

func (pc *PlanCommand) Run(args []string) int {
	cmd := pc.getCmd()
	cmd.SetArgs(args)
	err := cmd.Execute()
	if err != nil {
		pc.config.Logger.Error("error", err)
		pc.ui.Error(fmt.Sprintf("%s%s", ui.ERROR_PREFIX, color.RedString(" %v", err)))
		return 1
	}
	return 0
}

func (pc *PlanCommand) Synopsis() string {
	cmd := pc.getCmd()
	return cmd.Short
}

type planOpts struct {
	Scope                []string
	GlobalDeps           []string
	Since                string
	Ignore               []string
	Cwd                  string
	IncludeDepdendencies bool
	NoDependents         bool
	OutputJSON           bool
}

func (po *planOpts) ScopeOpts() *scope.Opts {

	// includeDependencies := false
	// if po.IncludeDepdendencies != nil {
	// 	includeDependencies = po.IncludeDepdendencies
	// } else if po.Since != "" && len(po.Scope) != 0 {
	// 	includeDependencies = true
	// }
	return &scope.Opts{
		IncludeDependencies: po.IncludeDepdendencies,
		IncludeDependents:   !po.NoDependents,
		Patterns:            po.Scope,
		Since:               po.Since,
		Cwd:                 po.Cwd,
		IgnorePatterns:      po.Ignore,
		GlobalDepPatterns:   po.GlobalDeps,
	}
}

func (pc *PlanCommand) getCmd() *cobra.Command {
	opts := &planOpts{}
	cmd := &cobra.Command{
		Use:   "turbo plan",
		Short: "Display packages affected by 'turbo run'",
		Long:  "Display the packages that would be affected by 'turbo run' with similar options",
		RunE: func(cmd *cobra.Command, args []string) error {
			// match the logic from run opts. If --since and --scope are specified,
			// assume --include-dependencies. If the user has specified --include-dependencies,
			// use the user's value.
			depsWasSet := cmd.Flags().Changed("include-dependencies")
			if !depsWasSet && opts.Since != "" && len(opts.Scope) != 0 {
				opts.IncludeDepdendencies = true
			}
			return plan(pc.config, pc.ui, opts)
		},
	}
	flags := cmd.Flags()
	flags.StringSliceVar(&opts.Scope, "scope", nil, "Specify package(s) to act as entry points for task\nexecution. Supports globs.")
	flags.StringSliceVar(&opts.GlobalDeps, "global-deps", nil, "Specify glob of global filesystem dependencies to\nbe hashed. Useful for .env and files in the root\ndirectory. Can be specified multiple times.")
	flags.StringVar(&opts.Since, "since", "", "Limit/Set scope to changed packages since a\nmergebase. This uses the git diff ${target_branch}...\nmechanism to identify which packages have changed.")
	flags.StringSliceVar(&opts.Ignore, "ignore", nil, "Files to ignore when calculating changed files\n(i.e. --since). Supports globs.")
	flags.BoolVar(&opts.IncludeDepdendencies, "include-dependencies", false, "Include the dependencies of tasks in execution.\n(default false)")
	flags.BoolVar(&opts.NoDependents, "no-deps", false, "Exclude dependent task consumers from execution.\n(default false)")
	flags.BoolVar(&opts.OutputJSON, "json", false, "If set, output the list of affected packages as a json array")
	// TODO(gsoltis): this should probably be a permanent flag
	flags.StringVar(&opts.Cwd, "cwd", "", "Which directory to run turbo in")
	flags.MarkHidden("cwd")
	return cmd
}

func plan(config *config.Config, tui cli.Ui, opts *planOpts) error {
	if opts.Cwd == "" {
		cwd, err := os.Getwd()
		if err != nil {
			return errors.Wrap(err, "failed to get cwd")
		}
		opts.Cwd = cwd
	}
	scopeOpts := opts.ScopeOpts()
	ctx, err := context.New(context.WithGraph(opts.Cwd, config))
	if err != nil {
		return err
	}

	scmInstance, err := scm.FromInRepo(scopeOpts.Cwd)
	if err != nil {
		if errors.Is(err, scm.ErrFallback) {
			config.Logger.Warn(err.Error())
		} else {
			return err
		}
	}
	var resolveUI cli.Ui
	if opts.OutputJSON {
		resolveUI = ui.NullUI
	} else {
		resolveUI = tui
	}
	pkgs, err := scope.ResolvePackages(scopeOpts, scmInstance, ctx, resolveUI, config.Logger)
	if err != nil {
		return err
	}
	pkgList := pkgs.UnsafeListOfStrings()
	sort.Strings(pkgList)
	if opts.OutputJSON {
		bytes, err := json.MarshalIndent(pkgList, "", "  ")
		if err != nil {
			return errors.Wrap(err, "failed to render to JSON")
		}
		tui.Output(string(bytes))
	} else {
		tui.Output("Packages in scope:")
		for _, pkg := range pkgList {
			tui.Output(fmt.Sprintf("  %v", pkg))
		}
	}
	return nil
}
