package run

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/context"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/scm"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/filter"
)

func resolvePackagesInScope(runOptions *RunOptions, ctx *context.Context, tui cli.Ui, logger hclog.Logger) (util.Set, error) {
	changedFiles, err := getChangedFiles(runOptions)
	if err != nil {
		return nil, err
	}

	// Note that we do this calculation *before* filtering the changed files.
	// The user can technically specify both that a file is a global dependency and
	// that it should be ignored, and currently we treat a change to that file as a
	// global change.
	hasRepoGlobalFileChanged, err := repoGlobalFileHasChanged(runOptions, changedFiles)
	if err != nil {
		return nil, err
	}
	filteredChangedFiles, err := filterIgnoredFiles(runOptions, changedFiles)
	if err != nil {
		return nil, err
	}

	changedPackages := make(util.Set)
	// Be specific with the changed packages only if no repo-wide changes occurred
	if !hasRepoGlobalFileChanged {
		changedPackages = getChangedPackages(filteredChangedFiles, ctx.PackageInfos)
	}

	// Scoped packages
	// Unwind scope globs
	scopePkgs, err := getScopedPackages(ctx.PackageNames, runOptions.scope)
	if err != nil {
		return nil, errors.Wrap(err, "invalid scope")
	}

	// Filter Packages
	filteredPkgs := make(util.Set)
	includeDependencies := runOptions.includeDependencies
	includeDependents := runOptions.includeDependents
	// If there has been a global change, changedPackages.Len() will be 0
	// If both scoped and since are specified, we have to merge two lists:
	// 1. changed packages that ARE themselves the scoped packages
	// 2. changed package consumers (package dependents) that are within the scoped subgraph
	if scopePkgs.Len() > 0 && changedPackages.Len() > 0 {
		filteredPkgs = scopePkgs.Intersection(changedPackages)
		for _, changed := range changedPackages {
			filteredPkgs.Add(changed)
			includeDependents = true

		}
		tui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s in scope: %s"), runOptions.since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if changedPackages.Len() > 0 {
		// --since was specified, there are changes, but no scope was specified.
		// Run the packages that have changed
		filteredPkgs = changedPackages
		tui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s: %s"), runOptions.since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if scopePkgs.Len() > 0 {
		// There was either a global change, or no changes, or no --since flag
		// There was a --scope flag, run the desired scopes
		filteredPkgs = scopePkgs
	} else if runOptions.since == "" {
		// No scope was specified, and no diff base was specified
		// Run every package
		for _, f := range ctx.PackageNames {
			filteredPkgs.Add(f)
		}
	}

	if includeDependents {
		// TODO(gsoltis): we're concurrently iterating and adding to a map, potentially
		// resulting in a bunch of duplicate work as we look for descendents of something
		// that has already had all of its descendents included.
		for _, pkg := range filteredPkgs {
			err = addDependents(filteredPkgs, pkg, ctx, logger)
			if err != nil {
				return nil, err
			}
		}
		logger.Debug("running with dependents")
	}

	if includeDependencies {
		// TODO(gsoltis): we're concurrently iterating and adding to a map, potentially
		// resulting in a bunch of duplicate work as we look for dependencies of something
		// that has already had all of its dependencies included.
		for _, pkg := range filteredPkgs {
			err = addDependencies(filteredPkgs, pkg, ctx, logger)
			if err != nil {
				return nil, err
			}
		}
		logger.Debug(ui.Dim("running with dependencies"))
	}
	return filteredPkgs, nil
}

func getChangedFiles(runOptions *RunOptions) ([]string, error) {
	if runOptions.since == "" {
		return []string{}, nil
	}
	gitRepoRoot, err := fs.FindupFrom(".git", runOptions.cwd)
	if err != nil {
		errors.Wrap(err, "Cannot find a .git folder in current working directory or in any parent directories. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo.")
	}
	git, err := scm.NewFallback(filepath.Dir(gitRepoRoot))
	if err != nil {
		return nil, err
	}
	return git.ChangedFiles(runOptions.since, true, runOptions.cwd), nil
}

func repoGlobalFileHasChanged(runOptions *RunOptions, changedFiles []string) (bool, error) {
	globalDepsGlob, err := filter.Compile(runOptions.globalDeps)
	if err != nil {
		return false, errors.Wrap(err, "invalid global deps glob")
	}

	if globalDepsGlob != nil {
		for _, f := range changedFiles {
			if globalDepsGlob.Match(f) {
				return true, nil
			}
		}
	}
	return false, nil
}

func filterIgnoredFiles(runOptions *RunOptions, changedFiles []string) (util.Set, error) {
	ignoreGlob, err := filter.Compile(runOptions.ignore)
	if err != nil {
		return nil, errors.Wrap(err, "invalid ignore globs")
	}
	filteredChanges := make(util.Set)
	for _, file := range changedFiles {
		// If we don't have anything to ignore, or if this file doesn't match the ignore pattern,
		// keep it as a changed file.
		if ignoreGlob == nil || !ignoreGlob.Match(file) {
			filteredChanges.Add(file)
		}
	}
	return filteredChanges, nil
}

func getChangedPackages(changedFiles util.Set, packageInfos map[interface{}]*fs.PackageJSON) util.Set {
	changedPackages := make(util.Set)
	for k, pkgInfo := range packageInfos {
		partialPath := pkgInfo.Dir
		if changedFiles.Some(func(v interface{}) bool {
			return strings.HasPrefix(fmt.Sprintf("%v", v), partialPath) // true
		}) {
			changedPackages.Add(k)
		}
	}
	return changedPackages
}

func addDependents(deps util.Set, pkg interface{}, ctx *context.Context, logger hclog.Logger) error {
	descenders, err := ctx.TopologicalGraph.Descendents(pkg)
	if err != nil {
		return errors.Wrap(err, "error calculating affected packages")
	}
	logger.Debug("dependents", "pkg", pkg, "value", descenders.List())
	for _, d := range descenders {
		// we need to exclude the fake root node
		// since it is not a real package
		if d != ctx.RootNode {
			deps.Add(d)
		}
	}
	return nil
}

func addDependencies(deps util.Set, pkg interface{}, ctx *context.Context, logger hclog.Logger) error {
	ancestors, err := ctx.TopologicalGraph.Ancestors(pkg)
	if err != nil {
		return errors.Wrap(err, "error getting dependency")
	}
	logger.Debug("dependencies", "pkg", pkg, "value", ancestors.List())
	for _, d := range ancestors {
		// we need to exclude the fake root node
		// since it is not a real package
		if d != ctx.RootNode {
			deps.Add(d)
		}
	}
	return nil
}
