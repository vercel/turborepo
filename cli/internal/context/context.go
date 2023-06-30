package context

import (
	"fmt"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/hashicorp/go-multierror"
	"github.com/vercel/turbo/cli/internal/core"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/workspace"

	"github.com/Masterminds/semver"
	"github.com/pyr-sh/dag"
	"golang.org/x/sync/errgroup"
)

// Warnings Error type for errors that don't prevent the creation of a functional Context
type Warnings struct {
	warns *multierror.Error
	mu    sync.Mutex
}

var _ error = (*Warnings)(nil)

func (w *Warnings) Error() string {
	return w.warns.Error()
}

func (w *Warnings) errorOrNil() error {
	if w.warns != nil {
		return w
	}
	return nil
}

func (w *Warnings) append(err error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.warns = multierror.Append(w.warns, err)
}

// Context of the CLI
type Context struct {
	// WorkspaceInfos contains the contents of package.json for every workspace
	// TODO(gsoltis): should the RootPackageJSON be included in WorkspaceInfos?
	WorkspaceInfos workspace.Catalog

	// WorkspaceNames is all the names of the workspaces
	WorkspaceNames []string

	// WorkspaceGraph is a graph of workspace dependencies
	// (based on package.json dependencies and devDependencies)
	WorkspaceGraph dag.AcyclicGraph

	// RootNode is a sigil identifying the root workspace
	RootNode string

	// Lockfile is a struct to read the lockfile based on the package manager
	Lockfile lockfile.Lockfile

	// PackageManager is an abstraction for all the info a package manager
	// can give us about the repo.
	PackageManager *packagemanager.PackageManager

	// Used to arbitrate access to the graph. We parallelise most build operations
	// and Go maps aren't natively threadsafe so this is needed.
	mutex sync.Mutex
}

// Splits "npm:^1.2.3" and "github:foo/bar.git" into a protocol part and a version part.
func parseDependencyProtocol(version string) (string, string) {
	parts := strings.Split(version, ":")
	if len(parts) == 1 {
		return "", parts[0]
	}

	return parts[0], strings.Join(parts[1:], ":")
}

func isProtocolExternal(protocol string) bool {
	// The npm protocol for yarn by default still uses the workspace package if the workspace
	// version is in a compatible semver range. See https://github.com/yarnpkg/berry/discussions/4015
	// For now, we will just assume if the npm protocol is being used and the version matches
	// its an internal dependency which matches the existing behavior before this additional
	// logic was added.

	// TODO: extend this to support the `enableTransparentWorkspaces` yarn option
	return protocol != "" && protocol != "npm"
}

func isWorkspaceReference(packageVersion string, dependencyVersion string, cwd string, rootpath string) bool {
	protocol, dependencyVersion := parseDependencyProtocol(dependencyVersion)

	if protocol == "workspace" {
		// TODO: Since support at the moment is non-existent for workspaces that contain multiple
		// versions of the same package name, just assume its a match and don't check the range
		// for an exact match.
		return true
	} else if protocol == "file" || protocol == "link" {
		abs, err := filepath.Abs(filepath.Join(cwd, dependencyVersion))
		if err != nil {
			// Default to internal if we have the package but somehow cannot get the path
			// TODO(gsoltis): log this?
			return true
		}
		isWithinRepo, err := fs.DirContainsPath(rootpath, filepath.FromSlash(abs))
		if err != nil {
			// Default to internal if we have the package but somehow cannot get the path
			// TODO(gsoltis): log this?
			return true
		}
		return isWithinRepo
	} else if isProtocolExternal(protocol) {
		// Other protocols are assumed to be external references ("github:", etc)
		return false
	} else if dependencyVersion == "*" {
		return true
	}

	// If we got this far, then we need to check the workspace package version to see it satisfies
	// the dependencies range to determin whether or not its an internal or external dependency.

	constraint, constraintErr := semver.NewConstraint(dependencyVersion)
	pkgVersion, packageVersionErr := semver.NewVersion(packageVersion)
	if constraintErr != nil || packageVersionErr != nil {
		// For backwards compatibility with existing behavior, if we can't parse the version then we
		// treat the dependency as an internal package reference and swallow the error.

		// TODO: some package managers also support tags like "latest". Does extra handling need to be
		// added for this corner-case
		return true
	}

	return constraint.Check(pkgVersion)
}

// SinglePackageGraph constructs a Context instance from a single package.
func SinglePackageGraph(rootPackageJSON *fs.PackageJSON, packageManagerName string) (*Context, error) {
	workspaceInfos := workspace.Catalog{
		PackageJSONs: map[string]*fs.PackageJSON{util.RootPkgName: rootPackageJSON},
		TurboConfigs: map[string]*fs.TurboJSON{},
	}
	c := &Context{
		WorkspaceInfos: workspaceInfos,
		RootNode:       core.ROOT_NODE_NAME,
	}
	c.WorkspaceGraph.Connect(dag.BasicEdge(util.RootPkgName, core.ROOT_NODE_NAME))
	packageManager, err := packagemanager.GetPackageManager(packageManagerName)
	if err != nil {
		return nil, err
	}
	c.PackageManager = packageManager
	return c, nil
}

// BuildPackageGraph constructs a Context instance with information about the package dependency graph
func BuildPackageGraph(repoRoot turbopath.AbsoluteSystemPath, rootPackageJSON *fs.PackageJSON, packageManagerName string) (*Context, error) {
	c := &Context{}
	rootpath := repoRoot.ToStringDuringMigration()
	c.WorkspaceInfos = workspace.Catalog{
		PackageJSONs: map[string]*fs.PackageJSON{},
		TurboConfigs: map[string]*fs.TurboJSON{},
	}
	c.RootNode = core.ROOT_NODE_NAME

	var warnings Warnings

	packageManager, err := packagemanager.GetPackageManager(packageManagerName)
	if err != nil {
		return nil, err
	}
	c.PackageManager = packageManager

	if err := c.resolveWorkspaceRootDeps(rootPackageJSON, &warnings); err != nil {
		// TODO(Gaspar) was this the intended return error?
		return nil, fmt.Errorf("could not resolve workspaces: %w", err)
	}

	// Get the workspaces from the package manager.
	// workspaces are absolute paths
	workspaces, err := c.PackageManager.GetWorkspaces(repoRoot)

	if err != nil {
		return nil, fmt.Errorf("workspace configuration error: %w", err)
	}

	// We will parse all package.json's simultaneously. We use a
	// wait group because we cannot fully populate the graph (the next step)
	// until all parsing is complete
	parseJSONWaitGroup := &errgroup.Group{}
	for _, workspace := range workspaces {
		pkgJSONPath := fs.UnsafeToAbsoluteSystemPath(workspace)
		parseJSONWaitGroup.Go(func() error {
			return c.parsePackageJSON(repoRoot, pkgJSONPath)
		})
	}

	if err := parseJSONWaitGroup.Wait(); err != nil {
		return nil, err
	}
	populateGraphWaitGroup := &errgroup.Group{}
	for _, pkg := range c.WorkspaceInfos.PackageJSONs {
		pkg := pkg
		populateGraphWaitGroup.Go(func() error {
			return c.populateWorkspaceGraphForPackageJSON(pkg, rootpath, pkg.Name, &warnings)
		})
	}

	if err := populateGraphWaitGroup.Wait(); err != nil {
		return nil, err
	}
	// Resolve dependencies for the root package. We override the vertexName in the graph
	// for the root package, since it can have an arbitrary name. We need it to have our
	// RootPkgName so that we can identify it as the root later on.
	err = c.populateWorkspaceGraphForPackageJSON(rootPackageJSON, rootpath, util.RootPkgName, &warnings)
	if err != nil {
		return nil, fmt.Errorf("failed to resolve dependencies for root package: %v", err)
	}
	c.WorkspaceInfos.PackageJSONs[util.RootPkgName] = rootPackageJSON

	if err := c.populateExternalDeps(repoRoot, rootPackageJSON, &warnings); err != nil {
		return nil, err
	}

	return c, warnings.errorOrNil()
}

func (c *Context) resolveWorkspaceRootDeps(rootPackageJSON *fs.PackageJSON, warnings *Warnings) error {
	pkg := rootPackageJSON
	pkg.UnresolvedExternalDeps = make(map[string]string)
	for dep, version := range pkg.DevDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.OptionalDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.Dependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	return nil
}

// populateWorkspaceGraphForPackageJSON fills in the edges for the dependencies of the given package
// that are within the monorepo, as well as collecting and hashing the dependencies of the package
// that are not within the monorepo. The vertexName is used to override the package name in the graph.
// This can happen when adding the root package, which can have an arbitrary name.
func (c *Context) populateWorkspaceGraphForPackageJSON(pkg *fs.PackageJSON, rootpath string, vertexName string, warnings *Warnings) error {
	c.mutex.Lock()
	defer c.mutex.Unlock()
	depMap := make(map[string]string)
	internalDepsSet := make(dag.Set)
	externalUnresolvedDepsSet := make(dag.Set)
	pkg.UnresolvedExternalDeps = make(map[string]string)

	for dep, version := range pkg.DevDependencies {
		depMap[dep] = version
	}

	for dep, version := range pkg.OptionalDependencies {
		depMap[dep] = version
	}

	for dep, version := range pkg.Dependencies {
		depMap[dep] = version
	}

	// split out internal vs. external deps
	for depName, depVersion := range depMap {
		if item, ok := c.WorkspaceInfos.PackageJSONs[depName]; ok && isWorkspaceReference(item.Version, depVersion, pkg.Dir.ToStringDuringMigration(), rootpath) {
			internalDepsSet.Add(depName)
			c.WorkspaceGraph.Connect(dag.BasicEdge(vertexName, depName))
		} else {
			externalUnresolvedDepsSet.Add(depName)
		}
	}

	for _, name := range externalUnresolvedDepsSet.List() {
		name := name.(string)
		if item, ok := pkg.DevDependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}

		if item, ok := pkg.OptionalDependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}

		if item, ok := pkg.Dependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}
	}

	// when there are no internal dependencies, we need to still add these leafs to the graph
	if internalDepsSet.Len() == 0 {
		c.WorkspaceGraph.Connect(dag.BasicEdge(pkg.Name, core.ROOT_NODE_NAME))
	}

	pkg.InternalDeps = make([]string, 0, internalDepsSet.Len())
	for _, v := range internalDepsSet.List() {
		pkg.InternalDeps = append(pkg.InternalDeps, fmt.Sprintf("%v", v))
	}

	sort.Strings(pkg.InternalDeps)

	return nil
}

func (c *Context) parsePackageJSON(repoRoot turbopath.AbsoluteSystemPath, pkgJSONPath turbopath.AbsoluteSystemPath) error {
	c.mutex.Lock()
	defer c.mutex.Unlock()

	if pkgJSONPath.FileExists() {
		pkg, err := fs.ReadPackageJSON(pkgJSONPath)
		if err != nil {
			return fmt.Errorf("parsing %s: %w", pkgJSONPath, err)
		}

		relativePkgJSONPath, err := repoRoot.PathTo(pkgJSONPath)
		if err != nil {
			return err
		}
		c.WorkspaceGraph.Add(pkg.Name)
		pkg.PackageJSONPath = turbopath.AnchoredSystemPathFromUpstream(relativePkgJSONPath)
		pkg.Dir = turbopath.AnchoredSystemPathFromUpstream(filepath.Dir(relativePkgJSONPath))
		if c.WorkspaceInfos.PackageJSONs[pkg.Name] != nil {
			existing := c.WorkspaceInfos.PackageJSONs[pkg.Name]
			return fmt.Errorf("Failed to add workspace \"%s\" from %s, it already exists at %s", pkg.Name, pkg.Dir, existing.Dir)
		}
		c.WorkspaceInfos.PackageJSONs[pkg.Name] = pkg
		c.WorkspaceNames = append(c.WorkspaceNames, pkg.Name)
	}
	return nil
}

func (c *Context) externalWorkspaceDeps() map[turbopath.AnchoredUnixPath]map[string]string {
	workspaces := make(map[turbopath.AnchoredUnixPath]map[string]string, len(c.WorkspaceInfos.PackageJSONs))
	for _, pkg := range c.WorkspaceInfos.PackageJSONs {
		workspaces[pkg.Dir.ToUnixPath()] = pkg.UnresolvedExternalDeps
	}
	return workspaces
}

func (c *Context) populateExternalDeps(repoRoot turbopath.AbsoluteSystemPath, rootPackageJSON *fs.PackageJSON, warnings *Warnings) error {
	if lockFile, err := c.PackageManager.ReadLockfile(repoRoot, rootPackageJSON); err != nil {
		warnings.append(err)
		rootPackageJSON.TransitiveDeps = nil
		rootPackageJSON.ExternalDepsHash = ""
	} else {
		c.Lockfile = lockFile
		if closures, err := lockfile.AllTransitiveClosures(c.externalWorkspaceDeps(), c.Lockfile); err != nil {
			warnings.append(err)
		} else {
			for _, pkg := range c.WorkspaceInfos.PackageJSONs {
				if closure, ok := closures[pkg.Dir.ToUnixPath()]; ok {
					if err := pkg.SetExternalDeps(closure); err != nil {
						return err
					}
				} else {
					return fmt.Errorf("Unable to calculate closure for workspace %s", pkg.Dir.ToString())
				}
			}
		}
	}

	return nil
}

// InternalDependencies finds all dependencies required by the slice of starting
// packages, as well as the starting packages themselves.
func (c *Context) InternalDependencies(start []string) ([]string, error) {
	vertices := make(dag.Set)
	for _, v := range start {
		vertices.Add(v)
	}
	s := make(dag.Set)
	memoFunc := func(v dag.Vertex, d int) error {
		s.Add(v)
		return nil
	}

	if err := c.WorkspaceGraph.DepthFirstWalk(vertices, memoFunc); err != nil {
		return nil, err
	}

	// Use for loop so we can coerce to string
	// .List() returns a list of interface{} types, but
	// we know they are strings.
	targets := make([]string, 0, s.Len())
	for _, dep := range s.List() {
		targets = append(targets, dep.(string))
	}
	sort.Strings(targets)

	return targets, nil
}

// ChangedPackages returns a list of changed packages based on the contents of a previous lockfile
// This assumes that none of the package.json in the workspace change, it is
// the responsibility of the caller to verify this.
func (c *Context) ChangedPackages(previousLockfile lockfile.Lockfile) ([]string, error) {
	if lockfile.IsNil(previousLockfile) || lockfile.IsNil(c.Lockfile) {
		return nil, fmt.Errorf("Cannot detect changed packages without previous and current lockfile")
	}

	closures, err := lockfile.AllTransitiveClosures(c.externalWorkspaceDeps(), previousLockfile)
	if err != nil {
		return nil, err
	}

	didPackageChange := func(pkgName string, pkg *fs.PackageJSON) bool {
		previousDeps, ok := closures[pkg.Dir.ToUnixPath()]
		if !ok || previousDeps.Cardinality() != len(pkg.TransitiveDeps) {
			return true
		}

		prevExternalDeps := make([]lockfile.Package, 0, previousDeps.Cardinality())
		for _, d := range previousDeps.ToSlice() {
			prevExternalDeps = append(prevExternalDeps, d.(lockfile.Package))
		}
		sort.Sort(lockfile.ByKey(prevExternalDeps))

		for i := range prevExternalDeps {
			if prevExternalDeps[i] != pkg.TransitiveDeps[i] {
				return true
			}
		}
		return false
	}

	changedPkgs := make([]string, 0, len(c.WorkspaceInfos.PackageJSONs))

	// check if prev and current have "global" changes e.g. lockfile bump
	globalChange := c.Lockfile.GlobalChange(previousLockfile)

	for pkgName, pkg := range c.WorkspaceInfos.PackageJSONs {
		if globalChange {
			break
		}
		if didPackageChange(pkgName, pkg) {
			if pkgName == util.RootPkgName {
				globalChange = true
			} else {
				changedPkgs = append(changedPkgs, pkgName)
			}
		}
	}

	if globalChange {
		changedPkgs = make([]string, 0, len(c.WorkspaceInfos.PackageJSONs))
		for pkgName := range c.WorkspaceInfos.PackageJSONs {
			changedPkgs = append(changedPkgs, pkgName)
		}
		sort.Strings(changedPkgs)
		return changedPkgs, nil
	}

	sort.Strings(changedPkgs)
	return changedPkgs, nil
}
