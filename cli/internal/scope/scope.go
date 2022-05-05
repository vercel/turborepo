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

type Opts struct {
	IncludeDependencies bool
	IncludeDependents   bool
	Patterns            []string
	Since               string
	Cwd                 string
	IgnorePatterns      []string
	GlobalDepPatterns   []string
	FilterPatterns      []string
}

// asFilterPatterns normalizes legacy selectors to filter syntax
func (o *Opts) asFilterPatterns() []string {
	patterns := make([]string, len(o.FilterPatterns))
	copy(patterns, o.FilterPatterns)
	prefix := ""
	if o.IncludeDependents {
		prefix = "..."
	}
	suffix := ""
	if o.IncludeDependencies {
		suffix = "..."
	}
	since := ""
	if o.Since != "" {
		since = fmt.Sprintf("[%v]", o.Since)
	}
	if len(o.Patterns) > 0 {
		// --scope implies our tweaked syntax to see if any dependency matches
		if since != "" {
			since = "..." + since
		}
		for _, pattern := range o.Patterns {
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

func ResolvePackages(opts *Opts, scm scm.SCM, ctx *context.Context, tui cli.Ui, logger hclog.Logger) (util.Set, error) {
	filterResolver := &scope_filter.Resolver{
		Graph:                &ctx.TopologicalGraph,
		PackageInfos:         ctx.PackageInfos,
		Cwd:                  opts.Cwd,
		PackagesChangedSince: opts.getPackageChangeFunc(scm, ctx.PackageInfos),
	}
	filterPatterns := opts.asFilterPatterns()
	filteredPkgs, err := filterResolver.GetPackagesFromPatterns(filterPatterns)
	if err != nil {
		return nil, err
	}

	if len(filterPatterns) == 0 {
		// no filters specified, run every package
		for _, f := range ctx.PackageNames {
			filteredPkgs.Add(f)
		}
	}
	filteredPkgs.Delete(ctx.RootNode)
	return filteredPkgs, nil
}

func (o *Opts) getPackageChangeFunc(scm scm.SCM, packageInfos map[interface{}]*fs.PackageJSON) scope_filter.PackagesChangedSince {
	return func(since string) (util.Set, error) {
		// We could filter changed files at the git level, since it's possible
		// that the changes we're interested in are scoped, but we need to handle
		// global dependencies changing as well. A future optimization might be to
		// scope changed files more deeply if we know there are no global dependencies.
		var changedFiles []string
		if since != "" {
			scmChangedFiles, err := scm.ChangedFiles(since, true, o.Cwd)
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
	for k, pkgInfo := range packageInfos {
		partialPath := pkgInfo.Dir
		if someFileHasPrefix(partialPath, changedFiles) {
			changedPackages.Add(k)
		}
	}
	return changedPackages
}

func someFileHasPrefix(prefix string, files []string) bool {
	for _, f := range files {
		if strings.HasPrefix(f, prefix) {
			return true
		}
	}
	return false
}
