package scope

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/scm"
	scope_filter "github.com/vercel/turbo/cli/internal/scope/filter"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/util/filter"
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

var _sinceHelp = `Limit/Set scope to changed packages since a
mergebase. This uses the git diff ${target_branch}...
mechanism to identify which packages have changed.`

func addLegacyFlagsFromArgs(opts *LegacyFilter, args *turbostate.ParsedArgsFromRust) {
	opts.IncludeDependencies = args.Command.Run.IncludeDependencies
	opts.SkipDependents = args.Command.Run.NoDeps
	opts.Entrypoints = args.Command.Run.Scope
	opts.Since = args.Command.Run.Since
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

var (
	_filterHelp = `Use the given selector to specify package(s) to act as
entry points. The syntax mirrors pnpm's syntax, and
additional documentation and examples can be found in
turbo's documentation https://turbo.build/repo/docs/reference/command-line-reference#--filter
--filter can be specified multiple times. Packages that
match any filter will be included.`
	_ignoreHelp    = `Files to ignore when calculating changed files (i.e. --since). Supports globs.`
	_globalDepHelp = `Specify glob of global filesystem dependencies to be hashed. Useful for .env and files
in the root directory. Includes turbo.json, root package.json, and the root lockfile by default.`
)

// OptsFromArgs adds the settings relevant to this package to the given Opts
func OptsFromArgs(opts *Opts, args *turbostate.ParsedArgsFromRust) {
	opts.FilterPatterns = args.Command.Run.Filter
	opts.IgnorePatterns = args.Command.Run.Ignore
	opts.GlobalDepPatterns = args.Command.Run.GlobalDeps
	addLegacyFlagsFromArgs(&opts.LegacyFilter, args)
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
		Graph:                  &ctx.WorkspaceGraph,
		WorkspaceInfos:         ctx.WorkspaceInfos,
		Cwd:                    cwd,
		PackagesChangedInRange: opts.getPackageChangeFunc(scm, cwd, ctx.WorkspaceInfos, ctx.PackageManager),
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
		for _, f := range ctx.WorkspaceNames {
			filteredPkgs.Add(f)
		}
	}
	filteredPkgs.Delete(ctx.RootNode)
	return filteredPkgs, isAllPackages, nil
}

func (o *Opts) getPackageChangeFunc(scm scm.SCM, cwd string, packageInfos graph.WorkspaceInfos, packageManager *packagemanager.PackageManager) scope_filter.PackagesChangedInRange {
	return func(fromRef string, toRef string) (util.Set, error) {
		// We could filter changed files at the git level, since it's possible
		// that the changes we're interested in are scoped, but we need to handle
		// global dependencies changing as well. A future optimization might be to
		// scope changed files more deeply if we know there are no global dependencies.
		var changedFiles []string
		if fromRef != "" {
			scmChangedFiles, err := scm.ChangedFiles(fromRef, toRef, true, cwd)
			if err != nil {
				return nil, err
			}
			changedFiles = scmChangedFiles
		}
		if hasRepoGlobalFileChanged, err := repoGlobalFileHasChanged(o, getDefaultGlobalDeps(packageManager), changedFiles); err != nil {
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

func getDefaultGlobalDeps(packageManager *packagemanager.PackageManager) []string {
	// include turbo.json, root package.json, and root lockfile as implicit global dependencies
	defaultGlobalDeps := []string{
		"turbo.json",
		"package.json",
	}
	if packageManager != nil {
		// TODO: we should be smarter here and determine if the lockfile changes actually impact the given scope
		defaultGlobalDeps = append(defaultGlobalDeps, packageManager.Lockfile)
	}

	return defaultGlobalDeps
}

func repoGlobalFileHasChanged(opts *Opts, defaultGlobalDeps []string, changedFiles []string) (bool, error) {
	globalDepsGlob, err := filter.Compile(append(opts.GlobalDepPatterns, defaultGlobalDeps...))
	if err != nil {
		return false, errors.Wrap(err, "invalid global deps glob")
	}

	if globalDepsGlob != nil {
		for _, file := range changedFiles {
			if globalDepsGlob.Match(filepath.ToSlash(file)) {
				return true, nil
			}
		}
	}
	return false, nil
}

func filterIgnoredFiles(opts *Opts, changedFiles []string) ([]string, error) {
	// changedFiles is an array of repo-relative system paths.
	// opts.IgnorePatterns is an array of unix-separator glob paths.
	ignoreGlob, err := filter.Compile(opts.IgnorePatterns)
	if err != nil {
		return nil, errors.Wrap(err, "invalid ignore globs")
	}
	filteredChanges := []string{}
	for _, file := range changedFiles {
		// If we don't have anything to ignore, or if this file doesn't match the ignore pattern,
		// keep it as a changed file.
		if ignoreGlob == nil || !ignoreGlob.Match(filepath.ToSlash(file)) {
			filteredChanges = append(filteredChanges, file)
		}
	}
	return filteredChanges, nil
}

func fileInPackage(changedFile string, packagePath string) bool {
	// This whole method is basically this regex: /^.*\/?$/
	// The regex is more-expensive, so we don't do it.

	// If it has the prefix, it might be in the package.
	if strings.HasPrefix(changedFile, packagePath) {
		// Now we need to see if the prefix stopped at a reasonable boundary.
		prefixLen := len(packagePath)
		changedFileLen := len(changedFile)

		// Same path.
		if prefixLen == changedFileLen {
			return true
		}

		// We know changedFile is longer than packagePath.
		// We can safely directly index into it.
		// Look ahead one byte and see if it's the separator.
		if changedFile[prefixLen] == os.PathSeparator {
			return true
		}
	}

	// If it does not have the prefix, it's definitely not in the package.
	return false
}

func getChangedPackages(changedFiles []string, packageInfos graph.WorkspaceInfos) util.Set {
	changedPackages := make(util.Set)
	for _, changedFile := range changedFiles {
		found := false
		for pkgName, pkgInfo := range packageInfos {
			if pkgName != util.RootPkgName && fileInPackage(changedFile, pkgInfo.Dir.ToStringDuringMigration()) {
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
