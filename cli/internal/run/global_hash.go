package run

import (
	"fmt"
	"path/filepath"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/globby"
	"github.com/vercel/turbo/cli/internal/hashing"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

const _globalCacheKey = "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo"

// Variables that we always include
var _defaultEnvVars = []string{
	"VERCEL_ANALYTICS_ID",
}

// GlobalHashable represents all the things that we use to create the global hash
type GlobalHashable struct {
	globalFileHashMap    map[turbopath.AnchoredUnixPath]string
	rootExternalDepsHash string
	envVars              env.DetailedMap
	globalCacheKey       string
	pipeline             fs.PristinePipeline
	envVarPassthroughs   []string
	envMode              util.EnvMode
}

// calculateGlobalHashFromHashable returns a hash string from the globalHashable
func calculateGlobalHashFromHashable(named GlobalHashable) (string, error) {
	// When we aren't in infer mode, we can hash the whole object
	if named.envMode != util.Infer {
		return fs.HashObject(named)
	}

	// In infer mode, if there is any passThru config (even if it is an empty array)
	// we'll hash the whole object, so we can detect changes to that config
	if named.envVarPassthroughs != nil {
		return fs.HashObject(named)
	}

	// If we're in infer mode, and there is no global pass through config,
	// we can use the old anonymous struct. this will be true for everyone not using the strict env
	// feature, and we don't want to break their cache.
	return fs.HashObject(getOldGlobalHashable(named))
}

// getOldGlobalHashable converts GlobalHashable into an anonymous struct.
// This exists because the global hash was originally implemented with an anonymous
// struct, and changing to a named struct changes the global hash (because the hash
// is essentially a hash of `fmt.Sprint("%#v", thing)`, and the type is part of that string.
// We keep this converter function around, because if we were to remove the anonymous
// struct, it would change the global hash for everyone, invalidating EVERY TURBO CACHE ON THE PLANET!
// We can remove this converter when we are going to have to update the global hash for something
// else anyway.
func getOldGlobalHashable(named GlobalHashable) struct {
	globalFileHashMap    map[turbopath.AnchoredUnixPath]string
	rootExternalDepsHash string
	hashedSortedEnvPairs env.EnvironmentVariablePairs
	globalCacheKey       string
	pipeline             fs.PristinePipeline
} {
	return struct {
		globalFileHashMap    map[turbopath.AnchoredUnixPath]string
		rootExternalDepsHash string
		hashedSortedEnvPairs env.EnvironmentVariablePairs
		globalCacheKey       string
		pipeline             fs.PristinePipeline
	}{
		globalFileHashMap:    named.globalFileHashMap,
		rootExternalDepsHash: named.rootExternalDepsHash,
		hashedSortedEnvPairs: named.envVars.All.ToHashable(),
		globalCacheKey:       named.globalCacheKey,
		pipeline:             named.pipeline,
	}
}

func calculateGlobalHash(
	rootpath turbopath.AbsoluteSystemPath,
	rootPackageJSON *fs.PackageJSON,
	pipeline fs.Pipeline,
	envVarDependencies []string,
	globalFileDependencies []string,
	packageManager *packagemanager.PackageManager,
	lockFile lockfile.Lockfile,
	envVarPassthroughs []string,
	envMode util.EnvMode,
	logger hclog.Logger,
) (GlobalHashable, error) {
	// Calculate env var dependencies
	envVars := []string{}
	envVars = append(envVars, envVarDependencies...)
	envVars = append(envVars, _defaultEnvVars...)
	globalHashableEnvVars, err := env.GetHashableEnvVars(envVars, []string{".*THASH.*"}, "")
	if err != nil {
		return GlobalHashable{}, err
	}

	logger.Debug("global hash env vars", "vars", globalHashableEnvVars.All.Names())

	// Calculate global file dependencies
	globalDeps := make(util.Set)
	if len(globalFileDependencies) > 0 {
		ignores, err := packageManager.GetWorkspaceIgnores(rootpath)
		if err != nil {
			return GlobalHashable{}, err
		}

		f, err := globby.GlobFiles(rootpath.ToStringDuringMigration(), globalFileDependencies, ignores)
		if err != nil {
			return GlobalHashable{}, err
		}

		for _, val := range f {
			globalDeps.Add(val)
		}
	}

	if lockFile == nil {
		// If we don't have lockfile information available, add the specfile and lockfile to global deps
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
		return GlobalHashable{}, fmt.Errorf("error hashing files: %w", err)
	}

	// Remove the passthroughs from hash consideration if we're explicitly loose.
	if envMode == util.Loose {
		envVarPassthroughs = nil
	}

	return GlobalHashable{
		globalFileHashMap:    globalFileHashMap,
		rootExternalDepsHash: rootPackageJSON.ExternalDepsHash,
		envVars:              globalHashableEnvVars,
		globalCacheKey:       _globalCacheKey,
		pipeline:             pipeline.Pristine(),
		envVarPassthroughs:   envVarPassthroughs,
		envMode:              envMode,
	}, nil
}
