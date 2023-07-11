// Package lockfile provides the lockfile interface and implementations for the various package managers
package lockfile

import (
	"fmt"
	"reflect"
	"sort"

	mapset "github.com/deckarep/golang-set"
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"golang.org/x/sync/errgroup"
)

// Lockfile Interface for general operations that work across all lockfiles
type Lockfile interface {
	// ResolvePackage Given a workspace, a package it imports and version returns the key, resolved version, and if it was found
	ResolvePackage(workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error)
	// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
	AllDependencies(key string) (map[string]string, bool)
	// GlobalChange checks if there are any differences between lockfiles that would completely invalidate
	// the cache.
	GlobalChange(other Lockfile) bool
}

// IsNil checks if lockfile is nil
func IsNil(l Lockfile) bool {
	return l == nil || reflect.ValueOf(l).IsNil()
}

// Package Structure representing a possible Pack
type Package struct {
	// Key used to lookup a package in the lockfile
	Key string `json:"key"`
	// The resolved version of a package as it appears in the lockfile
	Version string `json:"version"`
	// Set to true iff Key and Version are set
	Found bool `json:"-"`
}

// ByKey sort package structures by key
type ByKey []Package

func (p ByKey) Len() int {
	return len(p)
}

func (p ByKey) Swap(i, j int) {
	p[i], p[j] = p[j], p[i]
}

func (p ByKey) Less(i, j int) bool {
	if p[i].Key == p[j].Key {
		return p[i].Version < p[j].Version
	}

	return p[i].Key < p[j].Key
}

var _ (sort.Interface) = (*ByKey)(nil)

type closureMsg struct {
	workspace turbopath.AnchoredUnixPath
	closure   mapset.Set
}

// AllTransitiveClosures computes closures for all workspaces
func AllTransitiveClosures(
	workspaces map[turbopath.AnchoredUnixPath]map[string]string,
	lockFile Lockfile,
) (map[turbopath.AnchoredUnixPath]mapset.Set, error) {
	// We special case as Rust implementations have their own dep crawl
	if lf, ok := lockFile.(*NpmLockfile); ok {
		return rustTransitiveDeps(lf.contents, "npm", workspaces, nil)
	}
	if lf, ok := lockFile.(*BerryLockfile); ok {
		return rustTransitiveDeps(lf.contents, "berry", workspaces, lf.resolutions)
	}
	if lf, ok := lockFile.(*PnpmLockfile); ok {
		return rustTransitiveDeps(lf.contents, "pnpm", workspaces, nil)
	}
	if lf, ok := lockFile.(*YarnLockfile); ok {
		return rustTransitiveDeps(lf.contents, "yarn", workspaces, nil)
	}

	g := new(errgroup.Group)
	c := make(chan closureMsg, len(workspaces))
	closures := make(map[turbopath.AnchoredUnixPath]mapset.Set, len(workspaces))
	for workspace, deps := range workspaces {
		workspace := workspace
		deps := deps
		g.Go(func() error {
			closure, err := transitiveClosure(workspace, deps, lockFile)
			if err != nil {
				return err
			}
			c <- closureMsg{workspace: workspace, closure: closure}
			return nil
		})
	}
	err := g.Wait()
	close(c)
	if err != nil {
		return nil, err
	}
	for msg := range c {
		closures[msg.workspace] = msg.closure
	}
	return closures, nil
}

func transitiveClosure(
	workspaceDir turbopath.AnchoredUnixPath,
	unresolvedDeps map[string]string,
	lockFile Lockfile,
) (mapset.Set, error) {
	if IsNil(lockFile) {
		return nil, fmt.Errorf("No lockfile available to do analysis on")
	}

	resolvedPkgs := mapset.NewSet()
	lockfileEg := &errgroup.Group{}

	transitiveClosureHelper(lockfileEg, workspaceDir, lockFile, unresolvedDeps, resolvedPkgs)

	if err := lockfileEg.Wait(); err != nil {
		return nil, err
	}

	return resolvedPkgs, nil
}

func transitiveClosureHelper(
	wg *errgroup.Group,
	workspacePath turbopath.AnchoredUnixPath,
	lockfile Lockfile,
	unresolvedDirectDeps map[string]string,
	resolvedDeps mapset.Set,
) {
	for directDepName, unresolvedVersion := range unresolvedDirectDeps {
		directDepName := directDepName
		unresolvedVersion := unresolvedVersion
		wg.Go(func() error {

			lockfilePkg, err := lockfile.ResolvePackage(workspacePath, directDepName, unresolvedVersion)

			if err != nil {
				return err
			}

			if !lockfilePkg.Found || resolvedDeps.Contains(lockfilePkg) {
				return nil
			}

			resolvedDeps.Add(lockfilePkg)

			allDeps, ok := lockfile.AllDependencies(lockfilePkg.Key)

			if !ok {
				panic(fmt.Sprintf("Unable to find entry for %s", lockfilePkg.Key))
			}

			if len(allDeps) > 0 {
				transitiveClosureHelper(wg, workspacePath, lockfile, allDeps, resolvedDeps)
			}

			return nil
		})
	}
}

func rustTransitiveDeps(content []byte, packageManager string, workspaces map[turbopath.AnchoredUnixPath]map[string]string, resolutions map[string]string) (map[turbopath.AnchoredUnixPath]mapset.Set, error) {
	processedWorkspaces := make(map[string]map[string]string, len(workspaces))
	for workspacePath, workspace := range workspaces {
		processedWorkspaces[workspacePath.ToString()] = workspace
	}
	workspaceDeps, err := ffi.TransitiveDeps(content, packageManager, processedWorkspaces, resolutions)
	if err != nil {
		return nil, err
	}
	resolvedWorkspaces := make(map[turbopath.AnchoredUnixPath]mapset.Set, len(workspaceDeps))
	for workspace, dependencies := range workspaceDeps {
		depsSet := mapset.NewSet()
		for _, pkg := range dependencies.GetList() {
			depsSet.Add(Package{
				Found:   pkg.Found,
				Key:     pkg.Key,
				Version: pkg.Version,
			})
		}
		workspacePath := turbopath.AnchoredUnixPath(workspace)
		resolvedWorkspaces[workspacePath] = depsSet
	}
	return resolvedWorkspaces, nil
}
