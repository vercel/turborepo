package prune

import (
	"bufio"
	"fmt"

	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/vercel/turborepo/cli/internal/cache"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/ui"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

type opts struct {
	scope     string
	docker    bool
	outputDir string
}

func addPruneFlags(opts *opts, flags *pflag.FlagSet) {
	flags.StringVar(&opts.scope, "scope", "", "Specify package to act as entry point for pruned monorepo (required).")
	flags.BoolVar(&opts.docker, "docker", false, "Output pruned workspace into 'full' and 'json' directories optimized for Docker layer caching.")
	flags.StringVar(&opts.outputDir, "out-dir", "out", "Set the root directory for files output by this command")
	// No-op the cwd flag while the root level command is not yet cobra
	_ = flags.String("cwd", "", "")
	if err := flags.MarkHidden("cwd"); err != nil {
		// Fail fast if we have misconfigured our flags
		panic(err)
	}
}

// GetCmd returns the prune subcommand for use with cobra
func GetCmd(helper *cmdutil.Helper) *cobra.Command {
	opts := &opts{}
	cmd := &cobra.Command{
		Use:                   "prune --scope=<package name> [<flags>]",
		Short:                 "Prepare a subset of your monorepo.",
		SilenceUsage:          true,
		SilenceErrors:         true,
		DisableFlagsInUseLine: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			base, err := helper.GetCmdBase(cmd.Flags())
			if err != nil {
				return err
			}
			if opts.scope == "" {
				err := errors.New("at least one target must be specified")
				base.LogError(err.Error())
				return err
			}
			p := &prune{
				base,
			}
			if err := p.prune(opts); err != nil {
				logError(p.base.Logger, p.base.UI, err)
				return err
			}
			return nil
		},
	}
	addPruneFlags(opts, cmd.Flags())
	return cmd
}

func logError(logger hclog.Logger, ui cli.Ui, err error) {
	logger.Error("error", err)
	pref := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
	ui.Error(fmt.Sprintf("%s%s", pref, color.RedString(" %v", err)))
}

type prune struct {
	base *cmdutil.CmdBase
}

// Prune creates a smaller monorepo with only the required workspaces
func (p *prune) prune(opts *opts) error {
	cacheDir := cache.DefaultLocation(p.base.RepoRoot)
	rootPackageJSONPath := p.base.RepoRoot.Join("package.json")
	rootPackageJSON, err := fs.ReadPackageJSON(rootPackageJSONPath)
	if err != nil {
		return fmt.Errorf("failed to read package.json: %w", err)
	}
	ctx, err := context.New(context.WithGraph(p.base.RepoRoot, rootPackageJSON, cacheDir))
	if err != nil {
		return errors.Wrap(err, "could not construct graph")
	}
	p.base.Logger.Trace("scope", "value", opts.scope)
	target, scopeIsValid := ctx.PackageInfos[opts.scope]
	if !scopeIsValid {
		return errors.Errorf("invalid scope: package %v not found", opts.scope)
	}
	outDir := p.base.RepoRoot.Join(opts.outputDir)
	fullDir := outDir
	if opts.docker {
		fullDir = fullDir.Join("full")
	}

	p.base.Logger.Trace("target", "value", target.Name)
	p.base.Logger.Trace("directory", "value", target.Dir)
	p.base.Logger.Trace("external deps", "value", target.UnresolvedExternalDeps)
	p.base.Logger.Trace("internal deps", "value", target.InternalDeps)
	p.base.Logger.Trace("docker", "value", opts.docker)
	p.base.Logger.Trace("out dir", "value", outDir.ToString())

	canPrune, err := ctx.PackageManager.CanPrune(p.base.RepoRoot)
	if err != nil {
		return err
	}
	if !canPrune {
		return errors.Errorf("this command is not yet implemented for %s", ctx.PackageManager.Name)
	}

	p.base.UI.Output(fmt.Sprintf("Generating pruned monorepo for %v in %v", ui.Bold(opts.scope), ui.Bold(outDir.ToString())))

	packageJSONPath := outDir.Join("package.json")
	if err := packageJSONPath.EnsureDir(); err != nil {
		return errors.Wrap(err, "could not create output directory")
	}
	if workspacePath := ctx.PackageManager.WorkspaceConfigurationPath; workspacePath != "" && p.base.RepoRoot.Join(workspacePath).FileExists() {
		workspaceFile := fs.LstatCachedFile{Path: p.base.RepoRoot.Join(workspacePath)}
		if err := fs.CopyFile(&workspaceFile, outDir.Join(ctx.PackageManager.WorkspaceConfigurationPath).ToStringDuringMigration()); err != nil {
			return errors.Wrapf(err, "could not copy %s", ctx.PackageManager.WorkspaceConfigurationPath)
		}
	}
	workspaces := []turbopath.AnchoredSystemPath{}
	targets := []interface{}{opts.scope}
	internalDeps, err := ctx.TopologicalGraph.Ancestors(opts.scope)
	if err != nil {
		return errors.Wrap(err, "could find traverse the dependency graph to find topological dependencies")
	}
	targets = append(targets, internalDeps.List()...)

	lockfileKeys := make([]string, 0, len(rootPackageJSON.TransitiveDeps))
	lockfileKeys = append(lockfileKeys, rootPackageJSON.TransitiveDeps...)

	for _, internalDep := range targets {
		if internalDep == ctx.RootNode {
			continue
		}
		workspaces = append(workspaces, ctx.PackageInfos[internalDep].Dir)
		targetDir := fullDir.Join(ctx.PackageInfos[internalDep].Dir.ToStringDuringMigration())
		if err := targetDir.EnsureDir(); err != nil {
			return errors.Wrapf(err, "failed to create folder %v for %v", targetDir, internalDep)
		}
		if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].Dir.ToStringDuringMigration(), targetDir.ToStringDuringMigration()); err != nil {
			return errors.Wrapf(err, "failed to copy %v into %v", internalDep, targetDir)
		}
		if opts.docker {
			jsonDir := outDir.Join("json", ctx.PackageInfos[internalDep].PackageJSONPath.ToStringDuringMigration())
			if err := jsonDir.EnsureDir(); err != nil {
				return errors.Wrapf(err, "failed to create folder %v for %v", jsonDir, internalDep)
			}
			if err := fs.RecursiveCopy(ctx.PackageInfos[internalDep].PackageJSONPath.ToStringDuringMigration(), jsonDir.ToStringDuringMigration()); err != nil {
				return errors.Wrapf(err, "failed to copy %v into %v", internalDep, jsonDir)
			}
		}

		lockfileKeys = append(lockfileKeys, ctx.PackageInfos[internalDep].TransitiveDeps...)

		p.base.UI.Output(fmt.Sprintf(" - Added %v", ctx.PackageInfos[internalDep].Name))
	}
	p.base.Logger.Trace("new workspaces", "value", workspaces)
	if fs.FileExists(".gitignore") {
		if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.Join(".gitignore")}, fullDir.Join(".gitignore").ToStringDuringMigration()); err != nil {
			return errors.Wrap(err, "failed to copy root .gitignore")
		}
	}

	if fs.FileExists("turbo.json") {
		if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.Join("turbo.json")}, fullDir.Join("turbo.json").ToStringDuringMigration()); err != nil {
			return errors.Wrap(err, "failed to copy root turbo.json")
		}
	}

	if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.Join("package.json")}, fullDir.Join("package.json").ToStringDuringMigration()); err != nil {
		return errors.Wrap(err, "failed to copy root package.json")
	}

	if opts.docker {
		if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.Join("package.json")}, outDir.Join("json", "package.json").ToStringDuringMigration()); err != nil {
			return errors.Wrap(err, "failed to copy root package.json")
		}
	}

	lockfile, err := ctx.Lockfile.Subgraph(workspaces, lockfileKeys)
	if err != nil {
		return errors.Wrap(err, "Failed creating pruned lockfile")
	}

	if patches := lockfile.Patches(); patches != nil {
		for _, patch := range patches {
			if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.Join(patch)}, fullDir.Join(patch).ToStringDuringMigration()); err != nil {
				return errors.Wrap(err, "Failed copying patch file")
			}
		}
	}

	lockfilePath := outDir.Join(ctx.PackageManager.Lockfile)
	lockfileFile, err := lockfilePath.Create()
	if err != nil {
		return errors.Wrap(err, "Failed to create lockfile")
	}

	lockfileWriter := bufio.NewWriter(lockfileFile)
	if err := lockfile.Encode(lockfileWriter); err != nil {
		return errors.Wrap(err, "Failed to encode pruned lockfile")
	}

	if err := lockfileWriter.Flush(); err != nil {
		return errors.Wrap(err, "Failed to flush pruned lockfile")
	}

	return nil
}
