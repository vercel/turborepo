package context

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/core"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/hashing"
	"github.com/vercel/turborepo/cli/internal/packagemanager"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/Masterminds/semver"
	mapset "github.com/deckarep/golang-set"
	"github.com/pyr-sh/dag"
	"golang.org/x/sync/errgroup"
)

const _globalCacheKey = "Real G's move in silence like lasagna"

// Context of the CLI
type Context struct {
	// TODO(gsoltis): should the RootPackageJSON be included in PackageInfos?
	PackageInfos     map[interface{}]*fs.PackageJSON
	PackageNames     []string
	TopologicalGraph dag.AcyclicGraph
	RootNode         string
	GlobalHash       string
	Lockfile         *fs.YarnLockfile
	PackageManager   *packagemanager.PackageManager
	// Used to arbitrate access to the graph. We parallelise most build operations
	// and Go maps aren't natively threadsafe so this is needed.
	mutex sync.Mutex
}

// Option is used to configure context
type Option func(*Context) error

// New initializes run context
func New(opts ...Option) (*Context, error) {
	var m Context
	for _, opt := range opts {
		if err := opt(&m); err != nil {
			return nil, err
		}
	}

	return &m, nil
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

// WithGraph attaches information about the package dependency graph to the Context instance being
// constructed.
func WithGraph(config *config.Config, turboJSON *fs.TurboJSON, rootPackageJSON *fs.PackageJSON, cacheDir fs.AbsolutePath) Option {
	return func(c *Context) error {
		rootpath := config.Cwd.ToStringDuringMigration()
		c.PackageInfos = make(map[interface{}]*fs.PackageJSON)
		c.RootNode = core.ROOT_NODE_NAME

		if packageManager, err := packagemanager.GetPackageManager(config.Cwd, rootPackageJSON); err != nil {
			return err
		} else {
			c.PackageManager = packageManager
		}

		// this should go into the packagemanager abstraction
		if util.IsYarn(c.PackageManager.Name) {
			lockfile, err := fs.ReadLockfile(rootpath, c.PackageManager.Name, cacheDir)
			if err != nil {
				return fmt.Errorf("yarn.lock: %w", err)
			}
			c.Lockfile = lockfile
		}

		if err := c.resolveWorkspaceRootDeps(rootPackageJSON); err != nil {
			// TODO(Gaspar) was this the intended return error?
			return fmt.Errorf("could not resolve workspaces: %w", err)
		}

		// TODO: it seems like calculating the global hash could be separate from
		// construction of the package-dependency graph
		globalHash, err := calculateGlobalHash(
			config.Cwd,
			rootPackageJSON,
			turboJSON.Pipeline,
			turboJSON.GlobalDependencies,
			c.PackageManager,
			config.Logger,
			os.Environ(),
		)
		if err != nil {
			return fmt.Errorf("failed to calculate global hash: %v", err)
		}

		c.GlobalHash = globalHash

		// Get the workspaces from the package manager.
		workspaces, err := c.PackageManager.GetWorkspaces(config.Cwd)

		if err != nil {
			return fmt.Errorf("workspace configuration error: %w", err)
		}

		// We will parse all package.json's simultaneously. We use a
		// wait group because we cannot fully populate the graph (the next step)
		// until all parsing is complete
		parseJSONWaitGroup := &errgroup.Group{}
		for _, workspace := range workspaces {
			relativePkgPath, err := filepath.Rel(rootpath, workspace)
			if err != nil {
				return fmt.Errorf("non-nested package.json path %w", err)
			}
			parseJSONWaitGroup.Go(func() error {
				return c.parsePackageJSON(relativePkgPath)
			})
		}

		if err := parseJSONWaitGroup.Wait(); err != nil {
			return err
		}
		populateGraphWaitGroup := &errgroup.Group{}
		for _, pkg := range c.PackageInfos {
			pkg := pkg
			populateGraphWaitGroup.Go(func() error {
				return c.populateTopologicGraphForPackageJSON(pkg, rootpath, pkg.Name)
			})
		}

		if err := populateGraphWaitGroup.Wait(); err != nil {
			return err
		}
		// Resolve dependencies for the root package. We override the vertexName in the graph
		// for the root package, since it can have an arbitrary name. We need it to have our
		// RootPkgName so that we can identify it as the root later on.
		err = c.populateTopologicGraphForPackageJSON(rootPackageJSON, rootpath, util.RootPkgName)
		if err != nil {
			return fmt.Errorf("failed to resolve dependencies for root package: %v", err)
		}
		c.PackageInfos[util.RootPkgName] = rootPackageJSON

		return nil
	}
}

func (c *Context) resolveWorkspaceRootDeps(rootPackageJSON *fs.PackageJSON) error {
	seen := mapset.NewSet()
	var lockfileWg sync.WaitGroup
	pkg := rootPackageJSON
	depSet := mapset.NewSet()
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
	if util.IsYarn(c.PackageManager.Name) {
		pkg.SubLockfile = make(fs.YarnLockfile)
		c.resolveDepGraph(&lockfileWg, pkg.UnresolvedExternalDeps, depSet, seen, pkg)
		lockfileWg.Wait()
		pkg.ExternalDeps = make([]string, 0, depSet.Cardinality())
		for _, v := range depSet.ToSlice() {
			pkg.ExternalDeps = append(pkg.ExternalDeps, fmt.Sprintf("%v", v))
		}
		sort.Strings(pkg.ExternalDeps)
		hashOfExternalDeps, err := fs.HashObject(pkg.ExternalDeps)
		if err != nil {
			return err
		}
		pkg.ExternalDepsHash = hashOfExternalDeps
	} else {
		pkg.ExternalDeps = []string{}
		pkg.ExternalDepsHash = ""
	}

	return nil
}

// populateTopologicGraphForPackageJSON fills in the edges for the dependencies of the given package
// that are within the monorepo, as well as collecting and hashing the dependencies of the package
// that are not within the monorepo. The vertexName is used to override the package name in the graph.
// This can happen when adding the root package, which can have an arbitrary name.
func (c *Context) populateTopologicGraphForPackageJSON(pkg *fs.PackageJSON, rootpath string, vertexName string) error {
	c.mutex.Lock()
	defer c.mutex.Unlock()
	depMap := make(map[string]string)
	internalDepsSet := make(dag.Set)
	externalUnresolvedDepsSet := make(dag.Set)
	externalDepSet := mapset.NewSet()
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
		if item, ok := c.PackageInfos[depName]; ok && isWorkspaceReference(item.Version, depVersion, pkg.Dir, rootpath) {
			internalDepsSet.Add(depName)
			c.TopologicalGraph.Connect(dag.BasicEdge(vertexName, depName))
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

	pkg.SubLockfile = make(fs.YarnLockfile)
	seen := mapset.NewSet()
	var lockfileWg sync.WaitGroup
	c.resolveDepGraph(&lockfileWg, pkg.UnresolvedExternalDeps, externalDepSet, seen, pkg)
	lockfileWg.Wait()

	// when there are no internal dependencies, we need to still add these leafs to the graph
	if internalDepsSet.Len() == 0 {
		c.TopologicalGraph.Connect(dag.BasicEdge(pkg.Name, core.ROOT_NODE_NAME))
	}
	pkg.ExternalDeps = make([]string, 0, externalDepSet.Cardinality())
	for _, v := range externalDepSet.ToSlice() {
		pkg.ExternalDeps = append(pkg.ExternalDeps, fmt.Sprintf("%v", v))
	}
	pkg.InternalDeps = make([]string, 0, internalDepsSet.Len())
	for _, v := range internalDepsSet.List() {
		pkg.InternalDeps = append(pkg.InternalDeps, fmt.Sprintf("%v", v))
	}
	sort.Strings(pkg.InternalDeps)
	sort.Strings(pkg.ExternalDeps)
	hashOfExternalDeps, err := fs.HashObject(pkg.ExternalDeps)
	if err != nil {
		return err
	}
	pkg.ExternalDepsHash = hashOfExternalDeps
	return nil
}

func (c *Context) parsePackageJSON(buildFilePath string) error {
	c.mutex.Lock()
	defer c.mutex.Unlock()

	// log.Printf("[TRACE] reading package.json : %+v", buildFilePath)
	if fs.FileExists(buildFilePath) {
		pkg, err := fs.ReadPackageJSON(buildFilePath)
		if err != nil {
			return fmt.Errorf("parsing %s: %w", buildFilePath, err)
		}

		// log.Printf("[TRACE] adding %+v to graph", pkg.Name)
		c.TopologicalGraph.Add(pkg.Name)
		pkg.PackageJSONPath = buildFilePath
		pkg.Dir = filepath.Dir(buildFilePath)
		c.PackageInfos[pkg.Name] = pkg
		c.PackageNames = append(c.PackageNames, pkg.Name)
	}
	return nil
}

func (c *Context) resolveDepGraph(wg *sync.WaitGroup, unresolvedDirectDeps map[string]string, resolvedDepsSet mapset.Set, seen mapset.Set, pkg *fs.PackageJSON) {
	if !util.IsYarn(c.PackageManager.Name) {
		return
	}
	for directDepName, unresolvedVersion := range unresolvedDirectDeps {
		wg.Add(1)
		go func(directDepName, unresolvedVersion string) {
			defer wg.Done()
			var lockfileKey string
			lockfileKey1 := fmt.Sprintf("%v@%v", directDepName, unresolvedVersion)
			lockfileKey2 := fmt.Sprintf("%v@npm:%v", directDepName, unresolvedVersion)
			if seen.Contains(lockfileKey1) || seen.Contains(lockfileKey2) {
				return
			}

			seen.Add(lockfileKey1)
			seen.Add(lockfileKey2)

			var entry *fs.LockfileEntry
			entry1, ok1 := (*c.Lockfile)[lockfileKey1]
			entry2, ok2 := (*c.Lockfile)[lockfileKey2]
			if !ok1 && !ok2 {
				return
			}
			if ok1 {
				lockfileKey = lockfileKey1
				entry = entry1
			} else {
				lockfileKey = lockfileKey2
				entry = entry2
			}

			pkg.Mu.Lock()
			pkg.SubLockfile[lockfileKey] = entry
			pkg.Mu.Unlock()
			resolvedDepsSet.Add(fmt.Sprintf("%v@%v", directDepName, entry.Version))

			if len(entry.Dependencies) > 0 {
				c.resolveDepGraph(wg, entry.Dependencies, resolvedDepsSet, seen, pkg)
			}
			if len(entry.OptionalDependencies) > 0 {
				c.resolveDepGraph(wg, entry.OptionalDependencies, resolvedDepsSet, seen, pkg)
			}

		}(directDepName, unresolvedVersion)
	}
}

// getHashableTurboEnvVarsFromOs returns a list of environment variables names and
// that are safe to include in the global hash
func getHashableTurboEnvVarsFromOs(env []string) ([]string, []string) {
	var justNames []string
	var pairs []string
	for _, e := range env {
		kv := strings.SplitN(e, "=", 2)
		if strings.Contains(kv[0], "THASH") {
			justNames = append(justNames, kv[0])
			pairs = append(pairs, e)
		}
	}
	return justNames, pairs
}

// Variables that we always include
var _defaultEnvVars = []string{
	"VERCEL_ANALYTICS_ID",
}

func calculateGlobalHash(rootpath fs.AbsolutePath, rootPackageJSON *fs.PackageJSON, pipeline fs.Pipeline, externalGlobalDependencies []string, packageManager *packagemanager.PackageManager, logger hclog.Logger, env []string) (string, error) {
	// Calculate the global hash
	globalDeps := make(util.Set)

	globalHashableEnvNames := []string{}
	globalHashableEnvPairs := []string{}
	// Calculate global file and env var dependencies
	for _, builtinEnvVar := range _defaultEnvVars {
		globalHashableEnvNames = append(globalHashableEnvNames, builtinEnvVar)
		globalHashableEnvPairs = append(globalHashableEnvPairs, fmt.Sprintf("%v=%v", builtinEnvVar, os.Getenv(builtinEnvVar)))
	}
	if len(externalGlobalDependencies) > 0 {
		var globs []string
		for _, v := range externalGlobalDependencies {
			if strings.HasPrefix(v, "$") {
				trimmed := strings.TrimPrefix(v, "$")
				globalHashableEnvNames = append(globalHashableEnvNames, trimmed)
				globalHashableEnvPairs = append(globalHashableEnvPairs, fmt.Sprintf("%v=%v", trimmed, os.Getenv(trimmed)))
			} else {
				globs = append(globs, v)
			}
		}

		if len(globs) > 0 {
			ignores, err := packageManager.GetWorkspaceIgnores(rootpath)
			if err != nil {
				return "", err
			}

			f, err := globby.GlobFiles(rootpath.ToStringDuringMigration(), globs, ignores)
			if err != nil {
				return "", err
			}

			for _, val := range f {
				globalDeps.Add(val)
			}
		}
	}

	// get system env vars for hashing purposes, these include any variable that includes "TURBO"
	// that is NOT TURBO_TOKEN or TURBO_TEAM or TURBO_BINARY_PATH.
	names, pairs := getHashableTurboEnvVarsFromOs(env)
	globalHashableEnvNames = append(globalHashableEnvNames, names...)
	globalHashableEnvPairs = append(globalHashableEnvPairs, pairs...)
	// sort them for consistent hashing
	sort.Strings(globalHashableEnvNames)
	sort.Strings(globalHashableEnvPairs)
	logger.Debug("global hash env vars", "vars", globalHashableEnvNames)

	if !util.IsYarn(packageManager.Name) {
		// If we are not in Yarn, add the specfile and lockfile to global deps
		globalDeps.Add(filepath.Join(rootpath.ToStringDuringMigration(), packageManager.Specfile))
		globalDeps.Add(filepath.Join(rootpath.ToStringDuringMigration(), packageManager.Lockfile))
	}

	// No prefix, global deps already have full paths
	globalDepsArray := globalDeps.UnsafeListOfStrings()
	globalDepsPaths := make([]turbopath.AbsoluteSystemPath, len(globalDepsArray))
	for i, path := range globalDepsArray {
		globalDepsPaths[i] = turbopath.AbsoluteSystemPathFromUpstream(path)
	}

	globalFileHashMap, err := hashing.GetHashableDeps(rootpath, globalDepsPaths)
	if err != nil {
		return "", fmt.Errorf("error hashing files. make sure that git has been initialized %w", err)
	}
	globalHashable := struct {
		globalFileHashMap    map[turbopath.AnchoredUnixPath]string
		rootExternalDepsHash string
		hashedSortedEnvPairs []string
		globalCacheKey       string
		pipeline             fs.Pipeline
	}{
		globalFileHashMap:    globalFileHashMap,
		rootExternalDepsHash: rootPackageJSON.ExternalDepsHash,
		hashedSortedEnvPairs: globalHashableEnvPairs,
		globalCacheKey:       _globalCacheKey,
		pipeline:             pipeline,
	}
	globalHash, err := fs.HashObject(globalHashable)
	if err != nil {
		return "", fmt.Errorf("error hashing global dependencies %w", err)
	}
	return globalHash, nil
}
