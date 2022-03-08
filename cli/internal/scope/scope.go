package scope

import (
	"fmt"
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

type Opts struct {
	IncludeDependencies bool
	IncludeDependents   bool
	Patterns            []string
	Since               string
	Cwd                 string
	IgnorePatterns      []string
	GlobalDepPatterns   []string
}

func ResolvePackages(opts *Opts, scm scm.SCM, ctx *context.Context, tui cli.Ui, logger hclog.Logger) (util.Set, error) {
	changedFiles, err := getChangedFiles(opts, scm)
	if err != nil {
		return nil, err
	}

	// Note that we do this calculation *before* filtering the changed files.
	// The user can technically specify both that a file is a global dependency and
	// that it should be ignored, and currently we treat a change to that file as a
	// global change.
	hasRepoGlobalFileChanged, err := repoGlobalFileHasChanged(opts, changedFiles)
	if err != nil {
		return nil, err
	}
	filteredChangedFiles, err := filterIgnoredFiles(opts, changedFiles)
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
	scopePkgs, err := getScopedPackages(ctx.PackageNames, opts.Patterns)
	if err != nil {
		return nil, errors.Wrap(err, "invalid scope")
	}

	// Filter Packages
	filteredPkgs := make(util.Set)
	includeDependencies := opts.IncludeDependencies
	includeDependents := opts.IncludeDependents
	// If there has been a global change, run everything in scope
	// 		(this may be every package if no scope is provider)
	if hasRepoGlobalFileChanged {
		// If a global dependency has changed, run everything in scope.
		// If no scope was provided, run everything
		if scopePkgs.Len() > 0 {
			filteredPkgs = scopePkgs
		} else {
			for _, f := range ctx.PackageNames {
				filteredPkgs.Add(f)
			}
		}
	} else if scopePkgs.Len() > 0 && changedPackages.Len() > 0 {
		// If we have both a scope and changed packages:
		// We want the intersection of two sets:
		// 1. the scopes and all of their dependencies
		// 2. the changed packages and all of their dependents
		//
		// Note that other commandline flags can cause including dependents / dependencies
		// beyond this set

		// scopes and all deps
		rootsAndDeps := make(util.Set)
		for _, pkg := range scopePkgs {
			rootsAndDeps.Add(pkg)
			deps, err := ctx.TopologicalGraph.Ancestors(pkg)
			if err != nil {
				return nil, err
			}
			for _, dep := range deps {
				rootsAndDeps.Add(dep)
			}
		}

		// changed packages and all dependents
		for _, pkg := range changedPackages {
			// do the intersection inline, rather than building up the set
			if rootsAndDeps.Includes(pkg) {
				filteredPkgs.Add(pkg)
			}
			dependents, err := ctx.TopologicalGraph.Descendents(pkg)
			if err != nil {
				return nil, err
			}
			for _, dependent := range dependents {
				if rootsAndDeps.Includes(dependent) {
					filteredPkgs.Add(dependent)
				}
			}
		}
		tui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s in scope: %s"), opts.Since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if changedPackages.Len() > 0 {
		// --since was specified, there are changes, but no scope was specified.
		// Run the packages that have changed
		filteredPkgs = changedPackages
		tui.Output(fmt.Sprintf(ui.Dim("• Packages changed since %s: %s"), opts.Since, strings.Join(filteredPkgs.UnsafeListOfStrings(), ", ")))
	} else if scopePkgs.Len() > 0 {
		// There was either a global change, or no changes, or no --since flag
		// There was a --scope flag, run the desired scopes
		filteredPkgs = scopePkgs
	} else if opts.Since == "" {
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

	// ordered after includeDependents so that we pick up the dependencies of our dependents
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

func getChangedFiles(opts *Opts, scm scm.SCM) ([]string, error) {
	if opts.Since == "" {
		return []string{}, nil
	}
	return scm.ChangedFiles(opts.Since, true, opts.Cwd), nil
}

func repoGlobalFileHasChanged(opts *Opts, changedFiles []string) (bool, error) {
	globalDepsGlob, err := filter.Compile(opts.GlobalDepPatterns)
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

func filterIgnoredFiles(opts *Opts, changedFiles []string) (util.Set, error) {
	ignoreGlob, err := filter.Compile(opts.IgnorePatterns)
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

// getScopedPackages returns a set of package names in scope for a given list of glob patterns
func getScopedPackages(packageNames []string, scopePatterns []string) (util.Set, error) {
	scopedPkgs := make(util.Set)
	if len(scopePatterns) == 0 {
		return scopedPkgs, nil
	}

	include := make([]string, 0, len(scopePatterns))
	exclude := make([]string, 0, len(scopePatterns))

	for _, pattern := range scopePatterns {
		if strings.HasPrefix(pattern, "!") {
			exclude = append(exclude, pattern[1:])
		} else {
			include = append(include, pattern)
		}
	}

	glob, err := filter.NewIncludeExcludeFilter(include, exclude)
	if err != nil {
		return nil, err
	}
	for _, f := range packageNames {
		if glob.Match(f) {
			scopedPkgs.Add(f)
		}
	}

	if len(include) > 0 && scopedPkgs.Len() == 0 {
		return nil, errors.Errorf("No packages found matching the provided scope pattern.")
	}

	return scopedPkgs, nil
}
