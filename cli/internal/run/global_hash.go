package run

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/globby"
	"github.com/vercel/turborepo/cli/internal/hashing"
	"github.com/vercel/turborepo/cli/internal/lockfile"
	"github.com/vercel/turborepo/cli/internal/packagemanager"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
)

const _globalCacheKey = "Buffalo buffalo Buffalo buffalo buffalo buffalo Buffalo buffalo"

// Variables that we always include
var _defaultEnvVars = []string{
	"VERCEL_ANALYTICS_ID",
}

func calculateGlobalHash(rootpath turbopath.AbsoluteSystemPath, rootPackageJSON *fs.PackageJSON, pipeline fs.Pipeline, envVarDependencies []string, globalFileDependencies []string, packageManager *packagemanager.PackageManager, lockFile lockfile.Lockfile, logger hclog.Logger, env []string) (string, error) {
	// Calculate env var dependencies
	globalHashableEnvNames := []string{}
	globalHashableEnvPairs := []string{}
	for _, builtinEnvVar := range _defaultEnvVars {
		globalHashableEnvNames = append(globalHashableEnvNames, builtinEnvVar)
		globalHashableEnvPairs = append(globalHashableEnvPairs, fmt.Sprintf("%v=%v", builtinEnvVar, os.Getenv(builtinEnvVar)))
	}

	// Calculate global env var dependencies
	for _, v := range envVarDependencies {
		globalHashableEnvNames = append(globalHashableEnvNames, v)
		globalHashableEnvPairs = append(globalHashableEnvPairs, fmt.Sprintf("%v=%v", v, os.Getenv(v)))
	}

	// Calculate global file dependencies
	globalDeps := make(util.Set)
	if len(globalFileDependencies) > 0 {
		ignores, err := packageManager.GetWorkspaceIgnores(rootpath)
		if err != nil {
			return "", err
		}

		f, err := globby.GlobFiles(rootpath.ToStringDuringMigration(), globalFileDependencies, ignores)
		if err != nil {
			return "", err
		}

		for _, val := range f {
			globalDeps.Add(val)
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
		return "", fmt.Errorf("error hashing files: %w", err)
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
