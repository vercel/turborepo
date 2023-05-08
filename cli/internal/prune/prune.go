package prune

import (
	"bufio"
	"fmt"
	"os"
	"strings"

	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
)

type opts struct {
	scope     []string
	docker    bool
	outputDir string
}

// ExecutePrune executes the `prune` command.
func ExecutePrune(helper *cmdutil.Helper, executionState *turbostate.ExecutionState) error {
	base, err := helper.GetCmdBase(executionState)
	if err != nil {
		return err
	}
	if len(executionState.CLIArgs.Command.Prune.Scope) == 0 {
		err := errors.New("at least one target must be specified")
		base.LogError(err.Error())
		return err
	}
	p := &prune{
		base,
	}
	if err := p.prune(executionState.CLIArgs.Command.Prune, executionState.PackageManager); err != nil {
		logError(p.base.Logger, p.base.UI, err)
		return err
	}
	return nil
}

func logError(logger hclog.Logger, ui cli.Ui, err error) {
	logger.Error(fmt.Sprintf("error: %v", err))
	pref := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
	ui.Error(fmt.Sprintf("%s%s", pref, color.RedString(" %v", err)))
}

type prune struct {
	base *cmdutil.CmdBase
}

// Prune creates a smaller monorepo with only the required workspaces
func (p *prune) prune(opts *turbostate.PrunePayload, packageManagerName string) error {
	rootPackageJSONPath := p.base.RepoRoot.UntypedJoin("package.json")
	rootPackageJSON, err := fs.ReadPackageJSON(rootPackageJSONPath)
	if err != nil {
		return fmt.Errorf("failed to read package.json: %w", err)
	}
	ctx, err := context.BuildPackageGraph(p.base.RepoRoot, rootPackageJSON, packageManagerName)
	if err != nil {
		return errors.Wrap(err, "could not construct graph")
	}
	outDir := p.base.RepoRoot.UntypedJoin(opts.OutputDir)
	fullDir := outDir
	if opts.Docker {
		fullDir = fullDir.UntypedJoin("full")
	}

	p.base.Logger.Trace("scope", "value", strings.Join(opts.Scope, ", "))
	p.base.Logger.Trace("docker", "value", opts.Docker)
	p.base.Logger.Trace("out dir", "value", outDir.ToString())

	for _, scope := range opts.Scope {
		p.base.Logger.Trace("scope", "value", scope)
		target, scopeIsValid := ctx.WorkspaceInfos.PackageJSONs[scope]
		if !scopeIsValid {
			return errors.Errorf("invalid scope: package %v not found", scope)
		}
		p.base.Logger.Trace("target", "value", target.Name)
		p.base.Logger.Trace("directory", "value", target.Dir)
		p.base.Logger.Trace("external deps", "value", target.UnresolvedExternalDeps)
		p.base.Logger.Trace("internal deps", "value", target.InternalDeps)
	}

	canPrune, err := ctx.PackageManager.CanPrune(p.base.RepoRoot)
	if err != nil {
		return err
	}
	if !canPrune {
		return errors.Errorf("this command is not yet implemented for %s", ctx.PackageManager.Name)
	}
	if lockfile.IsNil(ctx.Lockfile) {
		return errors.New("Cannot prune without parsed lockfile")
	}

	p.base.UI.Output(fmt.Sprintf("Generating pruned monorepo for %v in %v", ui.Bold(strings.Join(opts.Scope, ", ")), ui.Bold(outDir.ToString())))

	packageJSONPath := outDir.UntypedJoin("package.json")
	if err := packageJSONPath.EnsureDir(); err != nil {
		return errors.Wrap(err, "could not create output directory")
	}
	if workspacePath := ctx.PackageManager.WorkspaceConfigurationPath; workspacePath != "" && p.base.RepoRoot.UntypedJoin(workspacePath).FileExists() {
		workspaceFile := fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(workspacePath)}
		if err := fs.CopyFile(&workspaceFile, outDir.UntypedJoin(ctx.PackageManager.WorkspaceConfigurationPath).ToStringDuringMigration()); err != nil {
			return errors.Wrapf(err, "could not copy %s", ctx.PackageManager.WorkspaceConfigurationPath)
		}
		if err := fs.CopyFile(&workspaceFile, fullDir.UntypedJoin(ctx.PackageManager.WorkspaceConfigurationPath).ToStringDuringMigration()); err != nil {
			return errors.Wrapf(err, "could not copy %s", ctx.PackageManager.WorkspaceConfigurationPath)
		}
		if opts.Docker {
			if err := fs.CopyFile(&workspaceFile, outDir.UntypedJoin("json", ctx.PackageManager.WorkspaceConfigurationPath).ToStringDuringMigration()); err != nil {
				return errors.Wrapf(err, "could not copy %s", ctx.PackageManager.WorkspaceConfigurationPath)
			}
		}
	}
	workspaces := []turbopath.AnchoredSystemPath{}
	targets, err := ctx.InternalDependencies(append(opts.Scope, util.RootPkgName))
	if err != nil {
		return errors.Wrap(err, "could not traverse the dependency graph to find topological dependencies")
	}
	p.base.Logger.Trace("targets", "value", targets)

	lockfileKeys := make([]string, 0, len(rootPackageJSON.TransitiveDeps))
	for _, pkg := range rootPackageJSON.TransitiveDeps {
		lockfileKeys = append(lockfileKeys, pkg.Key)
	}

	for _, internalDep := range targets {
		// We skip over the pseudo root node and the root package
		if internalDep == ctx.RootNode || internalDep == util.RootPkgName {
			continue
		}

		workspaces = append(workspaces, ctx.WorkspaceInfos.PackageJSONs[internalDep].Dir)
		originalDir := ctx.WorkspaceInfos.PackageJSONs[internalDep].Dir.RestoreAnchor(p.base.RepoRoot)
		info, err := originalDir.Lstat()
		if err != nil {
			return errors.Wrapf(err, "failed to lstat %s", originalDir)
		}
		targetDir := ctx.WorkspaceInfos.PackageJSONs[internalDep].Dir.RestoreAnchor(fullDir)
		if err := targetDir.MkdirAllMode(info.Mode()); err != nil {
			return errors.Wrapf(err, "failed to create folder %s for %v", targetDir, internalDep)
		}

		if err := fs.RecursiveCopy(ctx.WorkspaceInfos.PackageJSONs[internalDep].Dir.RestoreAnchor(p.base.RepoRoot), targetDir); err != nil {
			return errors.Wrapf(err, "failed to copy %v into %v", internalDep, targetDir)
		}
		if opts.Docker {
			jsonDir := outDir.UntypedJoin("json", ctx.WorkspaceInfos.PackageJSONs[internalDep].PackageJSONPath.ToStringDuringMigration())
			if err := jsonDir.EnsureDir(); err != nil {
				return errors.Wrapf(err, "failed to create folder %v for %v", jsonDir, internalDep)
			}
			if err := fs.RecursiveCopy(ctx.WorkspaceInfos.PackageJSONs[internalDep].PackageJSONPath.RestoreAnchor(p.base.RepoRoot), jsonDir); err != nil {
				return errors.Wrapf(err, "failed to copy %v into %v", internalDep, jsonDir)
			}
		}

		for _, pkg := range ctx.WorkspaceInfos.PackageJSONs[internalDep].TransitiveDeps {
			lockfileKeys = append(lockfileKeys, pkg.Key)
		}

		p.base.UI.Output(fmt.Sprintf(" - Added %v", ctx.WorkspaceInfos.PackageJSONs[internalDep].Name))
	}
	p.base.Logger.Trace("new workspaces", "value", workspaces)

	lockfile, err := ctx.Lockfile.Subgraph(workspaces, lockfileKeys)
	if err != nil {
		return errors.Wrap(err, "Failed creating pruned lockfile")
	}

	lockfilePath := outDir.UntypedJoin(ctx.PackageManager.Lockfile)
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

	if fs.FileExists(".gitignore") {
		if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(".gitignore")}, fullDir.UntypedJoin(".gitignore").ToStringDuringMigration()); err != nil {
			return errors.Wrap(err, "failed to copy root .gitignore")
		}
	}

	if fs.FileExists(".npmrc") {
		if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(".npmrc")}, fullDir.UntypedJoin(".npmrc").ToStringDuringMigration()); err != nil {
			return errors.Wrap(err, "failed to copy root .npmrc")
		}
		if opts.Docker {
			if err := fs.CopyFile(&fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(".npmrc")}, outDir.UntypedJoin("json/.npmrc").ToStringDuringMigration()); err != nil {
				return errors.Wrap(err, "failed to copy root .npmrc")
			}
		}
	}

	turboJSON, err := fs.LoadTurboConfig(p.base.RepoRoot, rootPackageJSON, false)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return errors.Wrap(err, "failed to read turbo.json")
	}
	if turboJSON != nil {
		// when executing a prune, it is not enough to simply copy the file, as
		// tasks may refer to scopes that no longer exist. to remedy this, we need
		// to remove from the Pipeline the TaskDefinitions that no longer apply
		for pipelineTask := range turboJSON.Pipeline {
			includeTask := false
			for _, includedPackage := range targets {
				if util.IsTaskInPackage(pipelineTask, includedPackage) {
					includeTask = true
					break
				}
			}

			if !includeTask {
				delete(turboJSON.Pipeline, pipelineTask)
			}
		}

		bytes, err := turboJSON.MarshalJSON()

		if err != nil {
			return errors.Wrap(err, "failed to write turbo.json")
		}

		if err := fullDir.UntypedJoin("turbo.json").WriteFile(bytes, 0644); err != nil {
			return errors.Wrap(err, "failed to prune workspace tasks from turbo.json")
		}
	}

	originalPackageJSON := fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin("package.json")}
	newPackageJSONPath := fullDir.UntypedJoin("package.json")
	// If the original lockfile uses any patches we rewrite the package.json to make sure it doesn't
	// include any patches that might have been pruned.
	if originalPatches := ctx.Lockfile.Patches(); originalPatches != nil {
		patches := lockfile.Patches()
		if err := ctx.PackageManager.PrunePatchedPackages(rootPackageJSON, patches); err != nil {
			return errors.Wrapf(err, "Unable to prune patches section of %s", rootPackageJSONPath)
		}
		packageJSONContent, err := fs.MarshalPackageJSON(rootPackageJSON)
		if err != nil {
			return err
		}

		info, err := originalPackageJSON.GetInfo()
		if err != nil {
			return err
		}
		newPackageJSON, err := newPackageJSONPath.Create()
		if err != nil {
			return err
		}
		if _, err := newPackageJSON.Write(packageJSONContent); err != nil {
			return err
		}
		if err := newPackageJSON.Chmod(info.Mode()); err != nil {
			return err
		}
		if err := newPackageJSON.Close(); err != nil {
			return err
		}

		for _, patch := range patches {
			if err := fs.CopyFile(
				&fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(patch.ToString())},
				fullDir.UntypedJoin(patch.ToString()).ToStringDuringMigration(),
			); err != nil {
				return errors.Wrap(err, "Failed copying patch file")
			}
			if opts.Docker {
				jsonDir := outDir.Join(turbopath.RelativeSystemPath("json"))
				if err := fs.CopyFile(
					&fs.LstatCachedFile{Path: p.base.RepoRoot.UntypedJoin(patch.ToString())},
					patch.ToSystemPath().RestoreAnchor(jsonDir).ToStringDuringMigration(),
				); err != nil {
					return errors.Wrap(err, "Failed copying patch file")
				}
			}
		}
	} else {
		if err := fs.CopyFile(
			&originalPackageJSON,
			fullDir.UntypedJoin("package.json").ToStringDuringMigration(),
		); err != nil {
			return errors.Wrap(err, "failed to copy root package.json")
		}
	}

	if opts.Docker {
		// Copy from the package.json in the full directory so we get the pruned version if needed
		if err := fs.CopyFile(
			&fs.LstatCachedFile{Path: newPackageJSONPath},
			outDir.Join(turbopath.RelativeUnixPath("json/package.json").ToSystemPath()).ToString(),
		); err != nil {
			return errors.Wrap(err, "failed to copy root package.json")
		}
	}

	return nil
}
