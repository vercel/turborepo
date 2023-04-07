package scope

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/context"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/scm"
	scope_filter "github.com/vercel/turbo/cli/internal/scope/filter"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/util/filter"
	"github.com/vercel/turbo/cli/internal/workspace"
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

	PackageInferenceRoot turbopath.RelativeSystemPath
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

// normalize package inference path. We compare against "" in several places, so maintain
// that behavior. In a post-rust-port world, this should more properly be an Option
func resolvePackageInferencePath(raw string) (turbopath.RelativeSystemPath, error) {
	pkgInferenceRoot, err := turbopath.CheckedToRelativeSystemPath(raw)
	if err != nil {
		return "", errors.Wrapf(err, "invalid package inference root %v", raw)
	}
	if pkgInferenceRoot == "." {
		return "", nil
	}
	return pkgInferenceRoot, nil
}

// OptsFromArgs adds the settings relevant to this package to the given Opts
func OptsFromArgs(opts *Opts, args *turbostate.ParsedArgsFromRust) error {
	opts.FilterPatterns = args.Command.Run.Filter
	opts.IgnorePatterns = args.Command.Run.Ignore
	opts.GlobalDepPatterns = args.Command.Run.GlobalDeps
	pkgInferenceRoot, err := resolvePackageInferencePath(args.Command.Run.PkgInferenceRoot)
	if err != nil {
		return err
	}
	opts.PackageInferenceRoot = pkgInferenceRoot
	addLegacyFlagsFromArgs(&opts.LegacyFilter, args)
	return nil
}

// AsFilterPatterns normalizes legacy selectors to filter syntax
func (l *LegacyFilter) AsFilterPatterns() []string {
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
func ResolvePackages(opts *Opts, repoRoot turbopath.AbsoluteSystemPath, scm scm.SCM, ctx *context.Context, tui cli.Ui, logger hclog.Logger) (util.Set, bool, error) {
	inferenceBase, err := calculateInference(repoRoot, opts.PackageInferenceRoot, ctx.WorkspaceInfos, logger)
	if err != nil {
		return nil, false, err
	}
	filterResolver := &scope_filter.Resolver{
		Graph:                  &ctx.WorkspaceGraph,
		WorkspaceInfos:         ctx.WorkspaceInfos,
		Cwd:                    repoRoot,
		Inference:              inferenceBase,
		PackagesChangedInRange: opts.getPackageChangeFunc(scm, repoRoot, ctx),
	}
	filterPatterns := opts.FilterPatterns
	legacyFilterPatterns := opts.LegacyFilter.AsFilterPatterns()
	filterPatterns = append(filterPatterns, legacyFilterPatterns...)
	isAllPackages := len(filterPatterns) == 0 && opts.PackageInferenceRoot == ""
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

func calculateInference(repoRoot turbopath.AbsoluteSystemPath, pkgInferencePath turbopath.RelativeSystemPath, packageInfos workspace.Catalog, logger hclog.Logger) (*scope_filter.PackageInference, error) {
	if pkgInferencePath == "" {
		// No inference specified, no need to calculate anything
		return nil, nil
	}
	logger.Debug(fmt.Sprintf("Using %v as a basis for selecting packages", pkgInferencePath))
	fullInferencePath := repoRoot.Join(pkgInferencePath)
	for _, pkgInfo := range packageInfos.PackageJSONs {
		pkgPath := pkgInfo.Dir.RestoreAnchor(repoRoot)
		inferredPathIsBelow, err := pkgPath.ContainsPath(fullInferencePath)
		if err != nil {
			return nil, err
		}
		// We skip over the root package as the inferred path will always be below it
		if inferredPathIsBelow && pkgPath != repoRoot {
			// set both. The user might have set a parent directory filter,
			// in which case we *should* fail to find any packages, but we should
			// do so in a consistent manner
			return &scope_filter.PackageInference{
				PackageName:   pkgInfo.Name,
				DirectoryRoot: pkgInferencePath,
			}, nil
		}
		inferredPathIsBetweenRootAndPkg, err := fullInferencePath.ContainsPath(pkgPath)
		if err != nil {
			return nil, err
		}
		if inferredPathIsBetweenRootAndPkg {
			// we've found *some* package below our inference directory. We can stop now and conclude
			// that we're looking for all packages in a subdirectory
			break
		}
	}
	return &scope_filter.PackageInference{
		DirectoryRoot: pkgInferencePath,
	}, nil
}

func (o *Opts) getPackageChangeFunc(scm scm.SCM, cwd turbopath.AbsoluteSystemPath, ctx *context.Context) scope_filter.PackagesChangedInRange {
	return func(fromRef string, toRef string) (util.Set, error) {
		// We could filter changed files at the git level, since it's possible
		// that the changes we're interested in are scoped, but we need to handle
		// global dependencies changing as well. A future optimization might be to
		// scope changed files more deeply if we know there are no global dependencies.
		var changedFiles []string
		if fromRef != "" {
			scmChangedFiles, err := scm.ChangedFiles(fromRef, toRef, cwd.ToStringDuringMigration())
			if err != nil {
				return nil, err
			}
			sort.Strings(scmChangedFiles)
			changedFiles = scmChangedFiles
		}
		makeAllPkgs := func() util.Set {
			allPkgs := make(util.Set)
			for pkg := range ctx.WorkspaceInfos.PackageJSONs {
				allPkgs.Add(pkg)
			}
			return allPkgs
		}
		if hasRepoGlobalFileChanged, err := repoGlobalFileHasChanged(o, getDefaultGlobalDeps(), changedFiles); err != nil {
			return nil, err
		} else if hasRepoGlobalFileChanged {
			return makeAllPkgs(), nil
		}

		filteredChangedFiles, err := filterIgnoredFiles(o, changedFiles)
		if err != nil {
			return nil, err
		}
		changedPkgs := getChangedPackages(filteredChangedFiles, ctx.WorkspaceInfos)

		if lockfileChanges, fullChanges := getChangesFromLockfile(scm, ctx, changedFiles, fromRef); !fullChanges {
			for _, pkg := range lockfileChanges {
				changedPkgs.Add(pkg)
			}
		} else {
			return makeAllPkgs(), nil
		}

		return changedPkgs, nil
	}
}

func getChangesFromLockfile(scm scm.SCM, ctx *context.Context, changedFiles []string, fromRef string) ([]string, bool) {
	lockfileFilter, err := filter.Compile([]string{ctx.PackageManager.Lockfile})
	if err != nil {
		panic(fmt.Sprintf("Lockfile is invalid glob: %v", err))
	}
	match := false
	for _, file := range changedFiles {
		if lockfileFilter.Match(file) {
			match = true
			break
		}
	}
	if !match {
		return nil, false
	}

	if lockfile.IsNil(ctx.Lockfile) {
		return nil, true
	}

	prevContents, err := scm.PreviousContent(fromRef, ctx.PackageManager.Lockfile)
	if err != nil {
		// unable to reconstruct old lockfile, assume everything changed
		return nil, true
	}
	prevLockfile, err := ctx.PackageManager.UnmarshalLockfile(ctx.WorkspaceInfos.PackageJSONs[util.RootPkgName], prevContents)
	if err != nil {
		// unable to parse old lockfile, assume everything changed
		return nil, true
	}
	additionalPkgs, err := ctx.ChangedPackages(prevLockfile)
	if err != nil {
		// missing at least one lockfile, assume everything changed
		return nil, true
	}

	return additionalPkgs, false
}

func getDefaultGlobalDeps() []string {
	// include turbo.json and root package.json as implicit global dependencies
	defaultGlobalDeps := []string{
		"turbo.json",
		"package.json",
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

func getChangedPackages(changedFiles []string, packageInfos workspace.Catalog) util.Set {
	changedPackages := make(util.Set)
	for _, changedFile := range changedFiles {
		found := false
		for pkgName, pkgInfo := range packageInfos.PackageJSONs {
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
