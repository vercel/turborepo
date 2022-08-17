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
	"github.com/vercel/turborepo/cli/internal/packagemanager"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"github.com/vercel/turborepo/cli/internal/util"
)

const _globalCacheKey = "Real G's move in silence like lasagna"

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
