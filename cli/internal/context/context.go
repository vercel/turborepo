package context

import (
	"fmt"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"turbo/internal/api"
	"turbo/internal/backends"
	"turbo/internal/config"
	"turbo/internal/core"
	"turbo/internal/fs"
	"turbo/internal/globby"
	"turbo/internal/util"

	mapset "github.com/deckarep/golang-set"
	"github.com/google/chrometracing"
	"github.com/pyr-sh/dag"
	gitignore "github.com/sabhiram/go-gitignore"
	"golang.org/x/sync/errgroup"
)

const GLOBAL_CACHE_KEY = "snozzberries"

// Context of the CLI
type Context struct {
	Args                   []string
	PackageInfos           map[interface{}]*fs.PackageJSON
	ColorCache             *ColorCache
	PackageNames           []string
	TopologicalGraph       dag.AcyclicGraph
	TaskGraph              dag.AcyclicGraph
	Dir                    string
	RootNode               string
	RootPackageJSON        *fs.PackageJSON
	GlobalHashableEnvPairs []string
	GlobalHashableEnvNames []string
	GlobalHash             string
	TraceFilePath          string
	Lockfile               *fs.YarnLockfile
	SCC                    [][]dag.Vertex
	Targets                []string
	Backend                *api.LanguageBackend
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

// WithArgs sets the arguments to the command that are used for parsing.
// Remaining arguments can be accessed using your flag set and asking for Args.
// Example: c.Flags().Args().
func WithArgs(args []string) Option {
	return func(c *Context) error {
		c.Args = args
		return nil
	}
}

func WithTracer(filename string) Option {
	return func(c *Context) error {
		if filename != "" {
			chrometracing.EnableTracing()
			c.TraceFilePath = filename
		}
		return nil
	}
}

func WithGraph(rootpath string, config *config.Config) Option {
	return func(c *Context) error {
		c.PackageInfos = make(map[interface{}]*fs.PackageJSON)
		c.ColorCache = NewColorCache()
		c.RootNode = core.ROOT_NODE_NAME
		// Need to ALWAYS have a root node, might as well do it now
		c.TaskGraph.Add(core.ROOT_NODE_NAME)

		cwd, err := os.Getwd()
		if err != nil {
			return fmt.Errorf("could not get cwd: %w", err)
		}

		pkg, err := fs.ReadPackageJSON("package.json")
		if err != nil {
			return fmt.Errorf("package.json: %w", err)
		}
		c.RootPackageJSON = pkg

		if backend, err := backends.GetBackend(cwd, pkg); err != nil {
			return err
		} else {
			c.Backend = backend
		}

		// this should go into the bacend abstraction
		if util.IsYarn(c.Backend.Name) {
			lockfile, err := fs.ReadLockfile(c.Backend.Name, config.Cache.Dir)
			if err != nil {
				return fmt.Errorf("yarn.lock: %w", err)
			}
			c.Lockfile = lockfile
		}

		if c.ResolveWorkspaceRootDeps() != nil {
			return err
		}

		spaces, err := c.Backend.GetWorkspaceGlobs()

		if err != nil {
			return fmt.Errorf("could not detect workspaces: %w", err)
		}

		// Calculate the global hash
		globalDeps := make(util.Set)

		// Calculate global file and env var dependencies
		if len(pkg.Turbo.GlobalDependencies) > 0 {
			var globs []string
			for _, v := range pkg.Turbo.GlobalDependencies {
				if strings.HasPrefix(v, "$") {
					trimmed := strings.TrimPrefix(v, "$")
					c.GlobalHashableEnvNames = append(c.GlobalHashableEnvNames, trimmed)
					c.GlobalHashableEnvPairs = append(c.GlobalHashableEnvPairs, fmt.Sprintf("%v=%v", trimmed, os.Getenv(trimmed)))
				} else {
					globs = append(globs, v)
				}
			}

			if len(globs) > 0 {
				f := globby.GlobFiles(rootpath, globs, []string{})
				for _, val := range f {
					globalDeps.Add(val)
				}
			}
		}

		// get system env vars for hashing purposes, these include any variable that includes "TURBO"
		// that is NOT TURBO_TOKEN or TURBO_TEAM or TURBO_BINARY_PATH.
		names, pairs := getHashableTurboEnvVarsFromOs()
		c.GlobalHashableEnvNames = append(c.GlobalHashableEnvNames, names...)
		c.GlobalHashableEnvPairs = append(c.GlobalHashableEnvPairs, pairs...)
		// sort them for consistent hashing
		sort.Strings(c.GlobalHashableEnvNames)
		sort.Strings(c.GlobalHashableEnvPairs)
		config.Logger.Debug("global hash env vars", "vars", c.GlobalHashableEnvNames)

		if !util.IsYarn(c.Backend.Name) {
			// If we are not in Yarn, add the specfile and lockfile to global deps
			globalDeps.Add(c.Backend.Specfile)
			globalDeps.Add(c.Backend.Lockfile)
		}

		globalFileHashMap, err := fs.GitHashForFiles(globalDeps.UnsafeListOfStrings(), rootpath)
		if err != nil {
			return fmt.Errorf("error hashing files. make sure that git has been initialized %w", err)
		}
		globalHashable := struct {
			globalFileHashMap    map[string]string
			rootExternalDepsHash string
			hashedSortedEnvPairs []string
			globalCacheKey       string
		}{
			globalFileHashMap:    globalFileHashMap,
			rootExternalDepsHash: pkg.ExternalDepsHash,
			hashedSortedEnvPairs: c.GlobalHashableEnvPairs,
			globalCacheKey:       GLOBAL_CACHE_KEY,
		}
		globalHash, err := fs.HashObject(globalHashable)
		if err != nil {
			return fmt.Errorf("error hashing global dependencies %w", err)
		}
		c.GlobalHash = globalHash
		targets, err := GetTargetsFromArguments(c.Args, &c.RootPackageJSON.Turbo)
		if err != nil {
			return err
		}
		c.Targets = targets
		// We will parse all package.json's simultaneously. We use a
		// waitgroup because we cannot fully populate the graph (the next step)
		// until all parsing is complete
		parseJSONWaitGroup := new(errgroup.Group)
		justJsons := make([]string, 0, len(spaces))
		for _, space := range spaces {
			justJsons = append(justJsons, path.Join(space, "package.json"))
		}

		f := globby.GlobFiles(rootpath, justJsons, getWorkspaceIgnores())

		for i, val := range f {
			_, val := i, val // https://golang.org/doc/faq#closures_and_goroutines
			parseJSONWaitGroup.Go(func() error {
				return c.parsePackageJSON(val)
			})
		}

		if err := parseJSONWaitGroup.Wait(); err != nil {
			return err
		}
		packageDepsHashGroup := new(errgroup.Group)
		populateGraphWaitGroup := new(errgroup.Group)
		for _, pkg := range c.PackageInfos {
			pkg := pkg
			populateGraphWaitGroup.Go(func() error {
				return c.populateTopologicGraphForPackageJson(pkg)
			})
			packageDepsHashGroup.Go(func() error {
				return c.loadPackageDepsHash(pkg)
			})
		}

		if err := populateGraphWaitGroup.Wait(); err != nil {
			return err
		}
		if err := packageDepsHashGroup.Wait(); err != nil {
			return err
		}

		// Only we can get the SCC (i.e. topological order)
		c.SCC = dag.StronglyConnected(&c.TopologicalGraph.Graph)
		return nil
	}
}

func (c *Context) loadPackageDepsHash(pkg *fs.PackageJSON) error {
	pkg.Mu.Lock()
	defer pkg.Mu.Unlock()
	hashObject, pkgDepsErr := fs.GetPackageDeps(&fs.PackageDepsOptions{
		PackagePath: pkg.Dir,
	})
	if pkgDepsErr != nil {
		hashObject = make(map[string]string)
		// Instead of implementing all gitignore properly, we hack it. We only respect .gitignore in the root and in
		// the directory of a package.
		ignore, err := safeCompileIgnoreFile(".gitignore")
		if err != nil {
			return err
		}

		ignorePkg, err := safeCompileIgnoreFile(filepath.Join(pkg.Dir, ".gitignore"))
		if err != nil {
			return err
		}

		fs.Walk(pkg.Dir, func(name string, isDir bool) error {
			rootMatch := ignore.MatchesPath(name)
			otherMatch := ignorePkg.MatchesPath(name)
			if !rootMatch && !otherMatch {
				if !isDir {
					hash, err := fs.GitLikeHashFile(name)
					if err != nil {
						return fmt.Errorf("could not hash file %v. \n%w", name, err)
					}
					hashObject[strings.TrimPrefix(name, pkg.Dir+"/")] = hash
				}
			}
			return nil
		})

		// ignorefile rules matched files
	}
	hashOfFiles, otherErr := fs.HashObject(hashObject)
	if otherErr != nil {
		return otherErr
	}
	pkg.FilesHash = hashOfFiles
	return nil
}

func (c *Context) ResolveWorkspaceRootDeps() (error) {
	seen := mapset.NewSet()
	var lockfileWg sync.WaitGroup
	pkg := c.RootPackageJSON
	depSet := mapset.NewSet()
	pkg.UnresolvedExternalDeps = make(map[string]string)
	for dep, version := range pkg.Dependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.DevDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.OptionalDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	for dep, version := range pkg.PeerDependencies {
		pkg.UnresolvedExternalDeps[dep] = version
	}
	if util.IsYarn(c.Backend.Name) {
		pkg.SubLockfile = make(fs.YarnLockfile)
		c.ResolveDepGraph(&lockfileWg, pkg.UnresolvedExternalDeps, depSet, seen, pkg)
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

// GetTargetsFromArguments returns a list of targets from the arguments and Turbo config.
// Return targets are always unique sorted alphabetically.
func GetTargetsFromArguments(arguments []string, configJson *fs.TurboConfigJSON) ([]string, error) {
	targets := make(util.Set)
	for _, arg := range arguments {
		if arg == "--" {
			break
		}
		if !strings.HasPrefix(arg, "-") {
			targets.Add(arg)
			found := false
			for task := range configJson.Pipeline {
				if task == arg {
					found = true
				}
			}
			if !found {
				return nil, fmt.Errorf("task `%v` not found in turbo pipeline in package.json. Are you sure you added it?", arg)
			}
		}
	}
	stringTargets := targets.UnsafeListOfStrings()
	sort.Strings(stringTargets)
	return stringTargets, nil
}

func (c *Context) populateTopologicGraphForPackageJson(pkg *fs.PackageJSON) error {
	c.mutex.Lock()
	defer c.mutex.Unlock()
	internalDepsSet := make(dag.Set)
	depSet := make(dag.Set)
	externalDepSet := mapset.NewSet()
	pkg.UnresolvedExternalDeps = make(map[string]string)
	for dep := range pkg.Dependencies {
		depSet.Add(dep)
	}

	for dep := range pkg.DevDependencies {
		depSet.Add(dep)
	}

	for dep := range pkg.OptionalDependencies {
		depSet.Add(dep)
	}

	for dep := range pkg.PeerDependencies {
		depSet.Add(dep)
	}

	// split out internal vs. external deps
	for _, dependencyName := range depSet.List() {
		if item, ok := c.PackageInfos[dependencyName]; ok {
			internalDepsSet.Add(item.Name)
			c.TopologicalGraph.Connect(dag.BasicEdge(pkg.Name, dependencyName))
		}
	}

	externalUnresolvedDepsSet := depSet.Difference(internalDepsSet)
	for _, name := range externalUnresolvedDepsSet.List() {
		name := name.(string)
		if item, ok := pkg.Dependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}

		if item, ok := pkg.DevDependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}

		if item, ok := pkg.OptionalDependencies[name]; ok {
			pkg.UnresolvedExternalDeps[name] = item
		}
	}

	pkg.SubLockfile = make(fs.YarnLockfile)
	seen := mapset.NewSet()
	var lockfileWg sync.WaitGroup
	c.ResolveDepGraph(&lockfileWg, pkg.UnresolvedExternalDeps, externalDepSet, seen, pkg)
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

func (c *Context) ResolveDepGraph(wg *sync.WaitGroup, unresolvedDirectDeps map[string]string, resolvedDepsSet mapset.Set, seen mapset.Set, pkg *fs.PackageJSON) {
	if !util.IsYarn(c.Backend.Name) {
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
				c.ResolveDepGraph(wg, entry.Dependencies, resolvedDepsSet, seen, pkg)
			}
			if len(entry.OptionalDependencies) > 0 {
				c.ResolveDepGraph(wg, entry.OptionalDependencies, resolvedDepsSet, seen, pkg)
			}

		}(directDepName, unresolvedVersion)
	}
}

func safeCompileIgnoreFile(filepath string) (*gitignore.GitIgnore, error) {
	if fs.FileExists(filepath) {
		return gitignore.CompileIgnoreFile(filepath)
	}
	// no op
	return gitignore.CompileIgnoreLines([]string{}...), nil
}

func getWorkspaceIgnores() []string {
	return []string{
		"**/node_modules/**/*",
		"**/bower_components/**/*",
		"**/test/**/*",
		"**/tests/**/*",
	}
}

// getHashableTurboEnvVarsFromOs returns a list of environment variables names and
// that are safe to include in the global hash
func getHashableTurboEnvVarsFromOs() ([]string, []string) {
	var justNames []string
	var pairs []string
	for _, e := range os.Environ() {
		kv := strings.SplitN(e, "=", 2)
		if strings.Contains(kv[0], "THASH") {
			justNames = append(justNames, kv[0])
			pairs = append(pairs, e)
		}
	}

	return justNames, pairs
}
