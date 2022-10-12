// package taskhash handles calculating dependency hashes for nodes in the task execution
// graph.

package taskhash

import (
	"fmt"
	"sort"
	"strings"
	"sync"

	"github.com/hashicorp/go-hclog"
	"github.com/pyr-sh/dag"
	gitignore "github.com/sabhiram/go-gitignore"
	"github.com/vercel/turborepo/cli/internal/doublestar"
	"github.com/vercel/turborepo/cli/internal/env"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/hashing"
	"github.com/vercel/turborepo/cli/internal/inference"
	"github.com/vercel/turborepo/cli/internal/nodes"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
	"golang.org/x/sync/errgroup"
)

// Tracker caches package-inputs hashes, as well as package-task hashes.
// package-inputs hashes must be calculated before package-task hashes,
// and package-task hashes must be calculated in topographical order.
// package-task hashing is threadsafe, provided topographical order is
// respected.
type Tracker struct {
	rootNode            string
	globalHash          string
	pipeline            fs.Pipeline
	packageInfos        map[interface{}]*fs.PackageJSON
	mu                  sync.RWMutex
	packageInputsHashes packageFileHashes
	packageTaskHashes   map[string]string // taskID -> hash
}

// NewTracker creates a tracker for package-inputs combinations and package-task combinations.
func NewTracker(rootNode string, globalHash string, pipeline fs.Pipeline, packageInfos map[interface{}]*fs.PackageJSON) *Tracker {
	return &Tracker{
		rootNode:          rootNode,
		globalHash:        globalHash,
		pipeline:          pipeline,
		packageInfos:      packageInfos,
		packageTaskHashes: make(map[string]string),
	}
}

// packageFileSpec defines a combination of a package and optional set of input globs
type packageFileSpec struct {
	pkg    string
	inputs []string
}

func specFromPackageTask(packageTask *nodes.PackageTask) packageFileSpec {
	return packageFileSpec{
		pkg:    packageTask.PackageName,
		inputs: packageTask.TaskDefinition.Inputs,
	}
}

// packageFileHashKey is a hashable representation of a packageFileSpec.
type packageFileHashKey string

// hashes the inputs for a packageTask
func (pfs packageFileSpec) ToKey() packageFileHashKey {
	sort.Strings(pfs.inputs)
	return packageFileHashKey(fmt.Sprintf("%v#%v", pfs.pkg, strings.Join(pfs.inputs, "!")))
}

func safeCompileIgnoreFile(filepath string) (*gitignore.GitIgnore, error) {
	if fs.FileExists(filepath) {
		return gitignore.CompileIgnoreFile(filepath)
	}
	// no op
	return gitignore.CompileIgnoreLines([]string{}...), nil
}

func (pfs *packageFileSpec) hash(pkg *fs.PackageJSON, repoRoot turbopath.AbsoluteSystemPath) (string, error) {
	hashObject, pkgDepsErr := hashing.GetPackageDeps(repoRoot, &hashing.PackageDepsOptions{
		PackagePath:   pkg.Dir,
		InputPatterns: pfs.inputs,
	})
	if pkgDepsErr != nil {
		manualHashObject, err := manuallyHashPackage(pkg, pfs.inputs, repoRoot)
		if err != nil {
			return "", err
		}
		hashObject = manualHashObject
	}
	hashOfFiles, otherErr := fs.HashObject(hashObject)
	if otherErr != nil {
		return "", otherErr
	}
	return hashOfFiles, nil
}

func manuallyHashPackage(pkg *fs.PackageJSON, inputs []string, rootPath turbopath.AbsoluteSystemPath) (map[turbopath.AnchoredUnixPath]string, error) {
	hashObject := make(map[turbopath.AnchoredUnixPath]string)
	// Instead of implementing all gitignore properly, we hack it. We only respect .gitignore in the root and in
	// the directory of a package.
	ignore, err := safeCompileIgnoreFile(rootPath.UntypedJoin(".gitignore").ToString())
	if err != nil {
		return nil, err
	}

	ignorePkg, err := safeCompileIgnoreFile(rootPath.UntypedJoin(pkg.Dir.ToStringDuringMigration(), ".gitignore").ToString())
	if err != nil {
		return nil, err
	}

	includePattern := ""
	if len(inputs) > 0 {
		includePattern = "{" + strings.Join(inputs, ",") + "}"
	}

	pathPrefix := rootPath.UntypedJoin(pkg.Dir.ToStringDuringMigration()).ToString()
	convertedPathPrefix := turbopath.AbsoluteSystemPathFromUpstream(pathPrefix)
	fs.Walk(pathPrefix, func(name string, isDir bool) error {
		convertedName := turbopath.AbsoluteSystemPathFromUpstream(name)
		rootMatch := ignore.MatchesPath(convertedName.ToString())
		otherMatch := ignorePkg.MatchesPath(convertedName.ToString())
		if !rootMatch && !otherMatch {
			if !isDir {
				if includePattern != "" {
					val, err := doublestar.PathMatch(includePattern, convertedName.ToString())
					if err != nil {
						return err
					}
					if !val {
						return nil
					}
				}
				hash, err := fs.GitLikeHashFile(convertedName.ToString())
				if err != nil {
					return fmt.Errorf("could not hash file %v. \n%w", convertedName.ToString(), err)
				}

				relativePath, err := convertedName.RelativeTo(convertedPathPrefix)
				if err != nil {
					return fmt.Errorf("File path cannot be made relative: %w", err)
				}
				hashObject[relativePath.ToUnixPath()] = hash
			}
		}
		return nil
	})
	return hashObject, nil
}

// packageFileHashes is a map from a package and optional input globs to the hash of
// the matched files in the package.
type packageFileHashes map[packageFileHashKey]string

// CalculateFileHashes hashes each unique package-inputs combination that is present
// in the task graph. Must be called before calculating task hashes.
func (th *Tracker) CalculateFileHashes(allTasks []dag.Vertex, workerCount int, repoRoot turbopath.AbsoluteSystemPath) error {
	hashTasks := make(util.Set)

	for _, v := range allTasks {
		taskID, ok := v.(string)
		if !ok {
			return fmt.Errorf("unknown task %v", taskID)
		}
		if taskID == th.rootNode {
			continue
		}
		pkgName, _ := util.GetPackageTaskFromId(taskID)
		if pkgName == th.rootNode {
			continue
		}

		taskDefinition, ok := th.pipeline.GetTaskDefinition(taskID)
		if !ok {
			return fmt.Errorf("missing pipeline entry %v", taskID)
		}

		pfs := &packageFileSpec{
			pkg:    pkgName,
			inputs: taskDefinition.Inputs,
		}

		hashTasks.Add(pfs)
	}

	hashes := make(map[packageFileHashKey]string)
	hashQueue := make(chan *packageFileSpec, workerCount)
	hashErrs := &errgroup.Group{}

	for i := 0; i < workerCount; i++ {
		hashErrs.Go(func() error {
			for packageFileSpec := range hashQueue {
				pkg, ok := th.packageInfos[packageFileSpec.pkg]
				if !ok {
					return fmt.Errorf("cannot find package %v", packageFileSpec.pkg)
				}
				hash, err := packageFileSpec.hash(pkg, repoRoot)
				if err != nil {
					return err
				}
				th.mu.Lock()
				pfsKey := packageFileSpec.ToKey()
				hashes[pfsKey] = hash
				th.mu.Unlock()
			}
			return nil
		})
	}
	for ht := range hashTasks {
		hashQueue <- ht.(*packageFileSpec)
	}
	close(hashQueue)
	err := hashErrs.Wait()
	if err != nil {
		return err
	}
	th.packageInputsHashes = hashes
	return nil
}

type taskHashInputs struct {
	hashOfFiles          string
	externalDepsHash     string
	task                 string
	outputs              fs.TaskOutputs
	passThruArgs         []string
	hashableEnvPairs     []string
	globalHash           string
	taskDependencyHashes []string
}

func (th *Tracker) calculateDependencyHashes(dependencySet dag.Set) ([]string, error) {
	dependencyHashSet := make(util.Set)

	rootPrefix := th.rootNode + util.TaskDelimiter
	th.mu.RLock()
	defer th.mu.RUnlock()
	for _, dependency := range dependencySet {
		if dependency == th.rootNode {
			continue
		}
		dependencyTask, ok := dependency.(string)
		if !ok {
			return nil, fmt.Errorf("unknown task: %v", dependency)
		}
		if strings.HasPrefix(dependencyTask, rootPrefix) {
			continue
		}
		dependencyHash, ok := th.packageTaskHashes[dependencyTask]
		if !ok {
			return nil, fmt.Errorf("missing hash for dependent task: %v", dependencyTask)
		}
		dependencyHashSet.Add(dependencyHash)
	}
	dependenciesHashList := dependencyHashSet.UnsafeListOfStrings()
	sort.Strings(dependenciesHashList)
	return dependenciesHashList, nil
}

// CalculateTaskHash calculates the hash for package-task combination. It is threadsafe, provided
// that it has previously been called on its task-graph dependencies. File hashes must be calculated
// first.
func (th *Tracker) CalculateTaskHash(packageTask *nodes.PackageTask, dependencySet dag.Set, logger hclog.Logger, args []string) (string, error) {
	pfs := specFromPackageTask(packageTask)
	pkgFileHashKey := pfs.ToKey()

	hashOfFiles, ok := th.packageInputsHashes[pkgFileHashKey]
	if !ok {
		return "", fmt.Errorf("cannot find package-file hash for %v", pkgFileHashKey)
	}

	var envPrefixes []string
	framework := inference.InferFramework(packageTask.Pkg)
	if framework != nil && framework.EnvPrefix != "" {
		// log auto detected framework and env prefix
		logger.Debug(fmt.Sprintf("auto detected framework for %s", packageTask.PackageName), "framework", framework.Slug, "env_prefix", framework.EnvPrefix)
		envPrefixes = append(envPrefixes, framework.EnvPrefix)
	}

	hashableEnvPairs := env.GetHashableEnvPairs(packageTask.TaskDefinition.EnvVarDependencies, envPrefixes)
	outputs := packageTask.HashableOutputs()
	taskDependencyHashes, err := th.calculateDependencyHashes(dependencySet)
	if err != nil {
		return "", err
	}
	// log any auto detected env vars
	logger.Debug(fmt.Sprintf("task hash env vars for %s:%s", packageTask.PackageName, packageTask.Task), "vars", hashableEnvPairs)

	hash, err := fs.HashObject(&taskHashInputs{
		hashOfFiles:          hashOfFiles,
		externalDepsHash:     packageTask.Pkg.ExternalDepsHash,
		task:                 packageTask.Task,
		outputs:              outputs,
		passThruArgs:         args,
		hashableEnvPairs:     hashableEnvPairs,
		globalHash:           th.globalHash,
		taskDependencyHashes: taskDependencyHashes,
	})
	if err != nil {
		return "", fmt.Errorf("failed to hash task %v: %v", packageTask.TaskID, hash)
	}
	th.mu.Lock()
	th.packageTaskHashes[packageTask.TaskID] = hash
	th.mu.Unlock()
	return hash, nil
}
