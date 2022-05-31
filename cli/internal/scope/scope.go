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
	scope_filter "github.com/vercel/turborepo/cli/internal/scope/filter"
	"github.com/vercel/turborepo/cli/internal/util"
	"github.com/vercel/turborepo/cli/internal/util/filter"
)

// LegacyFilter holds the options in use before the filter syntax. They have their own rules
// for how they are compiled into filter expressions.
type LegacyFilter struct {
	// IncludeDependencies is whether to include pkg.dependencies in execution (defaults to false)
	IncludeDependencies bool
	// SkipDependents is whether to skip dependent impacted consumers in execution (defaults to false)
	SkipDependents bool
	// Entrypoints is a list of package entrypoints
	Entrypoints []string
	// Since is the git ref used to calculate changed packages
	Since string
}

// Opts holds the options for how to select the entrypoint packages for a turbo run
type Opts struct {
	LegacyFilter LegacyFilter
	// IgnorePatterns is the list of globs of file paths to ignore from execution scope calculation
	IgnorePatterns []string
	// GlobalDepPatterns is a list of globs to global files whose contents will be included in the global hash calculation
	GlobalDepPatterns []string
	// Patterns are the filter patterns supplied to --filter on the commandline
	FilterPatterns []string
}

// asFilterPatterns normalizes legacy selectors to filter syntax
func (l *LegacyFilter) asFilterPatterns() []string {
	var patterns []string
	prefix := ""
	if !l.SkipDependents {
		prefix = "..."
	}
	suffix := ""
	if l.IncludeDependencies {
		suffix = "..."
	}
	since := ""
	if l.Since != "" {
		since = fmt.Sprintf("[%v]", l.Since)
	}
	if len(l.Entrypoints) > 0 {
		// --scope implies our tweaked syntax to see if any dependency matches
		if since != "" {
			since = "..." + since
		}
		for _, pattern := range l.Entrypoints {
			if strings.HasPrefix(pattern, "!") {
				patterns = append(patterns, pattern)
			} else {
				filterPattern := fmt.Sprintf("%v%v%v%v", prefix, pattern, since, suffix)
				patterns = append(patterns, filterPattern)
			}
		}
	} else if since != "" {
		// no scopes specified, but --since was provided
		filterPattern := fmt.Sprintf("%v%v%v", prefix, since, suffix)
		patterns = append(patterns, filterPattern)
	}
	return patterns
}

// ResolvePackages translates specified flags to a set of entry point packages for
// the selected tasks. Returns the selected packages and whether or not the selected
// packages represents a default "all packages".
func ResolvePackages(opts *Opts, cwd string, scm scm.SCM, ctx *context.Context, tui cli.Ui, logger hclog.Logger) (util.Set, bool, error) {
	filterResolver := &scope_filter.Resolver{
		Graph:                &ctx.TopologicalGraph,
		PackageInfos:         ctx.PackageInfos,
		Cwd:                  cwd,
		PackagesChangedSince: opts.getPackageChangeFunc(scm, cwd, ctx.PackageInfos),
	}
	filterPatterns := opts.FilterPatterns
	legacyFilterPatterns := opts.LegacyFilter.asFilterPatterns()
	filterPatterns = append(filterPatterns, legacyFilterPatterns...)
	isAllPackages := len(filterPatterns) == 0
	filteredPkgs, err := filterResolver.GetPackagesFromPatterns(filterPatterns)
	if err != nil {
		return nil, false, err
	}

	if isAllPackages {
		// no filters specified, run every package
		for _, f := range ctx.PackageNames {
			filteredPkgs.Add(f)
		}
	}
	filteredPkgs.Delete(ctx.RootNode)
	return filteredPkgs, isAllPackages, nil
}

func (o *Opts) getPackageChangeFunc(scm scm.SCM, cwd string, packageInfos map[interface{}]*fs.PackageJSON) scope_filter.PackagesChangedSince {
	// Note that "since" here is *not* o.LegacyFilter.Since. Each filter expression can have its own value for
	// "since", apart from the value specified via --since.
	return func(filterSince string) (util.Set, error) {
		// We could filter changed files at the git level, since it's possible
		// that the changes we're interested in are scoped, but we need to handle
		// global dependencies changing as well. A future optimization might be to
		// scope changed files more deeply if we know there are no global dependencies.
		var changedFiles []string
		if filterSince != "" {
			scmChangedFiles, err := scm.ChangedFiles(filterSince, true, cwd)
			if err != nil {
				return nil, err
			}
			changedFiles = scmChangedFiles
		}
		if hasRepoGlobalFileChanged, err := repoGlobalFileHasChanged(o, changedFiles); err != nil {
			return nil, err
		} else if hasRepoGlobalFileChanged {
			allPkgs := make(util.Set)
			for pkg := range packageInfos {
				allPkgs.Add(pkg)
			}
			return allPkgs, nil
		}
		filteredChangedFiles, err := filterIgnoredFiles(o, changedFiles)
		if err != nil {
			return nil, err
		}
		changedPkgs := getChangedPackages(filteredChangedFiles, packageInfos)
		return changedPkgs, nil
	}
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

func filterIgnoredFiles(opts *Opts, changedFiles []string) ([]string, error) {
	ignoreGlob, err := filter.Compile(opts.IgnorePatterns)
	if err != nil {
		return nil, errors.Wrap(err, "invalid ignore globs")
	}
	filteredChanges := []string{}
	for _, file := range changedFiles {
		// If we don't have anything to ignore, or if this file doesn't match the ignore pattern,
		// keep it as a changed file.
		if ignoreGlob == nil || !ignoreGlob.Match(file) {
			filteredChanges = append(filteredChanges, file)
		}
	}
	return filteredChanges, nil
}

func getChangedPackages(changedFiles []string, packageInfos map[interface{}]*fs.PackageJSON) util.Set {
	changedPackages := make(util.Set)
	for _, changedFile := range changedFiles {
		found := false
		for pkgName, pkgInfo := range packageInfos {
			if pkgName != util.RootPkgName && strings.HasPrefix(changedFile, pkgInfo.Dir) {
				changedPackages.Add(pkgName)
				found = true
				break
			}
		}
		if !found {
			// Consider the root package to have changed
			changedPackages.Add(util.RootPkgName)
		}
	}
	return changedPackages
}
