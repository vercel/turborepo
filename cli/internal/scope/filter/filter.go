package filter

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/pkg/errors"
	"github.com/pyr-sh/dag"
	"github.com/vercel/turbo/cli/internal/doublestar"
	"github.com/vercel/turbo/cli/internal/graph"
	"github.com/vercel/turbo/cli/internal/util"
)

type SelectedPackages struct {
	pkgs          util.Set
	unusedFilters []*TargetSelector
}

// PackagesChangedInRange is the signature of a function to provide the set of
// packages that have changed in a particular range of git refs.
type PackagesChangedInRange = func(fromRef string, toRef string) (util.Set, error)

type Resolver struct {
	Graph                  *dag.AcyclicGraph
	WorkspaceInfos         graph.WorkspaceInfos
	Cwd                    string
	PackagesChangedInRange PackagesChangedInRange
}

// GetPackagesFromPatterns compiles filter patterns and applies them, returning
// the selected packages
func (r *Resolver) GetPackagesFromPatterns(patterns []string) (util.Set, error) {
	selectors := []*TargetSelector{}
	for _, pattern := range patterns {
		selector, err := ParseTargetSelector(pattern, r.Cwd)
		if err != nil {
			return nil, err
		}
		selectors = append(selectors, &selector)
	}
	selected, err := r.GetFilteredPackages(selectors)
	if err != nil {
		return nil, err
	}
	return selected.pkgs, nil
}

func (r *Resolver) GetFilteredPackages(selectors []*TargetSelector) (*SelectedPackages, error) {
	prodPackageSelectors := []*TargetSelector{}
	allPackageSelectors := []*TargetSelector{}
	for _, selector := range selectors {
		if selector.followProdDepsOnly {
			prodPackageSelectors = append(prodPackageSelectors, selector)
		} else {
			allPackageSelectors = append(allPackageSelectors, selector)
		}
	}
	if len(allPackageSelectors) > 0 || len(prodPackageSelectors) > 0 {
		if len(allPackageSelectors) > 0 {
			selected, err := r.filterGraph(allPackageSelectors)
			if err != nil {
				return nil, err
			}
			return selected, nil
		}
	}
	return &SelectedPackages{
		pkgs: make(util.Set),
	}, nil
}

func (r *Resolver) filterGraph(selectors []*TargetSelector) (*SelectedPackages, error) {
	includeSelectors := []*TargetSelector{}
	excludeSelectors := []*TargetSelector{}
	for _, selector := range selectors {
		if selector.exclude {
			excludeSelectors = append(excludeSelectors, selector)
		} else {
			includeSelectors = append(includeSelectors, selector)
		}
	}
	var include *SelectedPackages
	if len(includeSelectors) > 0 {
		found, err := r.filterGraphWithSelectors(includeSelectors)
		if err != nil {
			return nil, err
		}
		include = found
	} else {
		vertexSet := make(util.Set)
		for _, v := range r.Graph.Vertices() {
			vertexSet.Add(v)
		}
		include = &SelectedPackages{
			pkgs: vertexSet,
		}
	}
	exclude, err := r.filterGraphWithSelectors(excludeSelectors)
	if err != nil {
		return nil, err
	}
	return &SelectedPackages{
		pkgs:          include.pkgs.Difference(exclude.pkgs),
		unusedFilters: append(include.unusedFilters, exclude.unusedFilters...),
	}, nil
}

func (r *Resolver) filterGraphWithSelectors(selectors []*TargetSelector) (*SelectedPackages, error) {
	unmatchedSelectors := []*TargetSelector{}

	cherryPickedPackages := make(dag.Set)
	walkedDependencies := make(dag.Set)
	walkedDependents := make(dag.Set)
	walkedDependentsDependencies := make(dag.Set)

	for _, selector := range selectors {
		// TODO(gsoltis): this should be a list?
		entryPackages, err := r.filterGraphWithSelector(selector)
		if err != nil {
			return nil, err
		}
		if entryPackages.Len() == 0 {
			unmatchedSelectors = append(unmatchedSelectors, selector)
		}
		for _, pkg := range entryPackages {
			if selector.includeDependencies {
				dependencies, err := r.Graph.Ancestors(pkg)
				if err != nil {
					return nil, errors.Wrapf(err, "failed to get dependencies of package %v", pkg)
				}
				for dep := range dependencies {
					walkedDependencies.Add(dep)
				}
				if !selector.excludeSelf {
					walkedDependencies.Add(pkg)
				}
			}
			if selector.includeDependents {
				dependents, err := r.Graph.Descendents(pkg)
				if err != nil {
					return nil, errors.Wrapf(err, "failed to get dependents of package %v", pkg)
				}
				for dep := range dependents {
					walkedDependents.Add(dep)
					if selector.includeDependencies {
						dependentDeps, err := r.Graph.Ancestors(dep)
						if err != nil {
							return nil, errors.Wrapf(err, "failed to get dependencies of dependent %v", dep)
						}
						for dependentDep := range dependentDeps {
							walkedDependentsDependencies.Add(dependentDep)
						}
					}
				}
				if !selector.excludeSelf {
					walkedDependents.Add(pkg)
				}
			}
			if !selector.includeDependencies && !selector.includeDependents {
				cherryPickedPackages.Add(pkg)
			}
		}
	}
	allPkgs := make(util.Set)
	for pkg := range cherryPickedPackages {
		allPkgs.Add(pkg)
	}
	for pkg := range walkedDependencies {
		allPkgs.Add(pkg)
	}
	for pkg := range walkedDependents {
		allPkgs.Add(pkg)
	}
	for pkg := range walkedDependentsDependencies {
		allPkgs.Add(pkg)
	}
	return &SelectedPackages{
		pkgs:          allPkgs,
		unusedFilters: unmatchedSelectors,
	}, nil
}

func (r *Resolver) filterGraphWithSelector(selector *TargetSelector) (util.Set, error) {
	if selector.matchDependencies {
		return r.filterSubtreesWithSelector(selector)
	}
	return r.filterNodesWithSelector(selector)
}

// filterNodesWithSelector returns the set of nodes that match a given selector
func (r *Resolver) filterNodesWithSelector(selector *TargetSelector) (util.Set, error) {
	entryPackages := make(util.Set)
	selectorWasUsed := false
	if selector.fromRef != "" {
		// get changed packaged
		selectorWasUsed = true
		changedPkgs, err := r.PackagesChangedInRange(selector.fromRef, selector.getToRef())
		if err != nil {
			return nil, err
		}
		parentDir := selector.parentDir
		for pkgName := range changedPkgs {
			if parentDir != "" {
				// Type assert/coerce to string here because we want to use
				// this value in a map that has string keys.
				// TODO(mehulkar) `changedPkgs` is a util.Set, we could make a `util.PackageNamesSet``
				// or something similar that is all strings.
				pkgNameStr := pkgName.(string)
				if pkgName == util.RootPkgName {
					// The root package changed, only add it if
					// the parentDir is equivalent to the root
					if matches, err := doublestar.PathMatch(parentDir, r.Cwd); err != nil {
						return nil, fmt.Errorf("failed to resolve directory relationship %v contains %v: %v", parentDir, r.Cwd, err)
					} else if matches {
						entryPackages.Add(pkgName)
					}
				} else if pkg, ok := r.WorkspaceInfos[pkgNameStr]; !ok {
					return nil, fmt.Errorf("missing info for package %v", pkgName)
				} else if matches, err := doublestar.PathMatch(parentDir, filepath.Join(r.Cwd, pkg.Dir.ToStringDuringMigration())); err != nil {
					return nil, fmt.Errorf("failed to resolve directory relationship %v contains %v: %v", selector.parentDir, pkg.Dir, err)
				} else if matches {
					entryPackages.Add(pkgName)
				}
			} else {
				entryPackages.Add(pkgName)
			}
		}
	} else if selector.parentDir != "" {
		// get packages by path
		selectorWasUsed = true
		parentDir := selector.parentDir
		if parentDir == r.Cwd {
			entryPackages.Add(util.RootPkgName)
		} else {
			for name, pkg := range r.WorkspaceInfos {
				if matches, err := doublestar.PathMatch(parentDir, filepath.Join(r.Cwd, pkg.Dir.ToStringDuringMigration())); err != nil {
					return nil, fmt.Errorf("failed to resolve directory relationship %v contains %v: %v", selector.parentDir, pkg.Dir, err)
				} else if matches {
					entryPackages.Add(name)
				}
			}
		}
	}
	if selector.namePattern != "" {
		// find packages that match name
		if !selectorWasUsed {
			matched, err := matchPackageNamesToVertices(selector.namePattern, r.Graph.Vertices())
			if err != nil {
				return nil, err
			}
			entryPackages = matched
			selectorWasUsed = true
		} else {
			matched, err := matchPackageNames(selector.namePattern, entryPackages)
			if err != nil {
				return nil, err
			}
			entryPackages = matched
		}
	}
	// TODO(gsoltis): we can do this earlier
	// Check if the selector specified anything
	if !selectorWasUsed {
		return nil, fmt.Errorf("invalid selector: %v", selector.raw)
	}
	return entryPackages, nil
}

// filterSubtreesWithSelector returns the set of nodes where the node or any of its dependencies
// match a selector
func (r *Resolver) filterSubtreesWithSelector(selector *TargetSelector) (util.Set, error) {
	// foreach package that matches parentDir && namePattern, check if any dependency is in changed packages
	changedPkgs, err := r.PackagesChangedInRange(selector.fromRef, selector.getToRef())
	if err != nil {
		return nil, err
	}

	parentDir := ""
	if selector.parentDir != "" {
		parentDir = filepath.Join(r.Cwd, selector.parentDir)
	}
	entryPackages := make(util.Set)
	for name, pkg := range r.WorkspaceInfos {
		if parentDir == "" {
			entryPackages.Add(name)
		} else if matches, err := doublestar.PathMatch(parentDir, pkg.Dir.ToStringDuringMigration()); err != nil {
			return nil, fmt.Errorf("failed to resolve directory relationship %v contains %v: %v", selector.parentDir, pkg.Dir, err)
		} else if matches {
			entryPackages.Add(name)
		}
	}
	if selector.namePattern != "" {
		matched, err := matchPackageNames(selector.namePattern, entryPackages)
		if err != nil {
			return nil, err
		}
		entryPackages = matched
	}
	roots := make(util.Set)
	matched := make(util.Set)
	for pkg := range entryPackages {
		if matched.Includes(pkg) {
			roots.Add(pkg)
			continue
		}
		deps, err := r.Graph.Ancestors(pkg)
		if err != nil {
			return nil, err
		}
		for changedPkg := range changedPkgs {
			if !selector.excludeSelf && pkg == changedPkg {
				roots.Add(pkg)
				break
			}
			if deps.Include(changedPkg) {
				roots.Add(pkg)
				matched.Add(changedPkg)
				break
			}
		}
	}
	return roots, nil
}

func matchPackageNamesToVertices(pattern string, vertices []dag.Vertex) (util.Set, error) {
	packages := make(util.Set)
	for _, v := range vertices {
		packages.Add(v)
	}
	packages.Add(util.RootPkgName)
	return matchPackageNames(pattern, packages)
}

func matchPackageNames(pattern string, packages util.Set) (util.Set, error) {
	matcher, err := matcherFromPattern(pattern)
	if err != nil {
		return nil, err
	}
	matched := make(util.Set)
	for _, pkg := range packages {
		pkg := pkg.(string)
		if matcher(pkg) {
			matched.Add(pkg)
		}
	}
	if matched.Len() == 0 && !strings.HasPrefix(pattern, "@") && !strings.Contains(pattern, "/") {
		// we got no matches and the pattern isn't a scoped package.
		// Check if we have exactly one scoped package that does match
		scopedPattern := fmt.Sprintf("@*/%v", pattern)
		matcher, err = matcherFromPattern(scopedPattern)
		if err != nil {
			return nil, err
		}
		foundScopedPkg := false
		for _, pkg := range packages {
			pkg := pkg.(string)
			if matcher(pkg) {
				if foundScopedPkg {
					// we found a second scoped package. Return the empty set, we can't
					// disambiguate
					return make(util.Set), nil
				}
				foundScopedPkg = true
				matched.Add(pkg)
			}
		}
	}
	return matched, nil
}
