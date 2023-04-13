package run

import (
	"fmt"
	"path/filepath"
	"strings"

	"github.com/hashicorp/go-hclog"
	"github.com/mitchellh/cli"
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

// This exists because the global hash used to have different fields. Changing
// to a new struct layout changes the global hash. We can remove this converter
// when we are going to have to update the global hash for something else.
type oldGlobalHashable struct {
	globalFileHashMap    map[turbopath.AnchoredUnixPath]string
	rootExternalDepsHash string
	envVars              env.EnvironmentVariablePairs
	globalCacheKey       string
	pipeline             fs.PristinePipeline
}

// calculateGlobalHashFromHashable returns a hash string from the globalHashable
func calculateGlobalHashFromHashable(full GlobalHashable) (string, error) {
	switch full.envMode {
	case util.Infer:
		if full.envVarPassthroughs != nil {
			// In infer mode, if there is any passThru config (even if it is an empty array)
			// we'll hash the whole object, so we can detect changes to that config
			// Further, resolve the envMode to the concrete value.
			full.envMode = util.Strict
			return fs.HashObject(full)
		}

		// If we're in infer mode, and there is no global pass through config,
		// we use the old struct layout. this will be true for everyone not using the strict env
		// feature, and we don't want to break their cache.
		return fs.HashObject(oldGlobalHashable{
			globalFileHashMap:    full.globalFileHashMap,
			rootExternalDepsHash: full.rootExternalDepsHash,
			envVars:              full.envVars.All.ToHashable(),
			globalCacheKey:       full.globalCacheKey,
			pipeline:             full.pipeline,
		})
	case util.Loose:
		// Remove the passthroughs from hash consideration if we're explicitly loose.
		full.envVarPassthroughs = nil
		return fs.HashObject(full)
	case util.Strict:
		// Collapse `nil` and `[]` in strict mode.
		if full.envVarPassthroughs == nil {
			full.envVarPassthroughs = make([]string, 0)
		}
		return fs.HashObject(full)
	default:
		panic("unimplemented environment mode")
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
	ui cli.Ui,
	isStructuredOutput bool,
) (GlobalHashable, error) {
	// Calculate env var dependencies
	envVars := []string{}
	envVars = append(envVars, envVarDependencies...)
	envVars = append(envVars, _defaultEnvVars...)
	globalHashableEnvVars, err := env.GetHashableEnvVars(envVars, []string{".*THASH.*"}, "")
	if err != nil {
		return GlobalHashable{}, err
	}

	// The only way we can add env vars into the hash via matching is via THASH,
	// so we only do a simple check here for entries in `BySource.Matching`.
	// If we enable globalEnv to accept wildcard characters, we'll need to update this
	// check.
	if !isStructuredOutput && len(globalHashableEnvVars.BySource.Matching) > 0 {
		ui.Warn(fmt.Sprintf("[DEPRECATED] Using .*THASH.* to specify an environment variable for inclusion into the hash is deprecated. You specified: %s.", strings.Join(globalHashableEnvVars.BySource.Matching.Names(), ", ")))
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
