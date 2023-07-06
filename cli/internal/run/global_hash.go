package run

import (
	"fmt"
	"path/filepath"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/fs/hash"
	"github.com/vercel/turbo/cli/internal/globby"
	"github.com/vercel/turbo/cli/internal/hashing"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/packagemanager"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
)

const _globalCacheKey = "You don't understand! I coulda had class. I coulda been a contender. I could've been somebody, instead of a bum, which is what I am."

// Variables that we always include
var _defaultEnvVars = []string{
	"VERCEL_ANALYTICS_ID",
}

// GlobalHashableInputs represents all the things that we use to create the global hash
type GlobalHashableInputs struct {
	globalCacheKey       string
	globalFileHashMap    map[turbopath.AnchoredUnixPath]string
	rootExternalDepsHash string
	env                  []string
	resolvedEnvVars      env.DetailedMap
	passThroughEnv       []string
	envMode              util.EnvMode
	frameworkInference   bool
	dotEnv               turbopath.AnchoredUnixPathArray
}

// calculateGlobalHash is a transformation of GlobalHashableInputs.
// It's used for the situations where we have an `EnvMode` specified
// as that is not compatible with existing global hashes.
func calculateGlobalHash(full GlobalHashableInputs) (string, error) {
	return fs.HashGlobal(hash.GlobalHashable{
		GlobalCacheKey:       full.globalCacheKey,
		GlobalFileHashMap:    full.globalFileHashMap,
		RootExternalDepsHash: full.rootExternalDepsHash,
		Env:                  full.env,
		ResolvedEnvVars:      full.resolvedEnvVars.All.ToHashable(),
		PassThroughEnv:       full.passThroughEnv,
		EnvMode:              full.envMode,
		FrameworkInference:   full.frameworkInference,
		DotEnv:               full.dotEnv,
	})
}

// calculateGlobalHashFromHashableInputs returns a hash string from the GlobalHashableInputs
func calculateGlobalHashFromHashableInputs(full GlobalHashableInputs) (string, error) {
	switch full.envMode {
	case util.Infer:
		if full.passThroughEnv != nil {
			// In infer mode, if there is any passThru config (even if it is an empty array)
			// we'll hash the whole object, so we can detect changes to that config
			// Further, resolve the envMode to the concrete value.
			full.envMode = util.Strict
		}

		return calculateGlobalHash(full)
	case util.Loose:
		// Remove the passthroughs from hash consideration if we're explicitly loose.
		full.passThroughEnv = nil
		return calculateGlobalHash(full)
	case util.Strict:
		// Collapse `nil` and `[]` in strict mode.
		if full.passThroughEnv == nil {
			full.passThroughEnv = make([]string, 0)
		}
		return calculateGlobalHash(full)
	default:
		panic("unimplemented environment mode")
	}
}

func getGlobalHashInputs(
	logger hclog.Logger,
	rootpath turbopath.AbsoluteSystemPath,
	rootPackageJSON *fs.PackageJSON,
	packageManager *packagemanager.PackageManager,
	lockFile lockfile.Lockfile,
	globalFileDependencies []string,
	envAtExecutionStart env.EnvironmentVariableMap,
	globalEnv []string,
	globalPassThroughEnv []string,
	envMode util.EnvMode,
	frameworkInference bool,
	dotEnv turbopath.AnchoredUnixPathArray,
) (GlobalHashableInputs, error) {
	globalHashableEnvVars, err := getGlobalHashableEnvVars(envAtExecutionStart, globalEnv)
	if err != nil {
		return GlobalHashableInputs{}, err
	}

	logger.Debug("global hash env vars", "vars", globalHashableEnvVars.All.Names())

	// Calculate global file dependencies
	globalDeps := make(util.Set)
	if len(globalFileDependencies) > 0 {
		ignores, err := packageManager.GetWorkspaceIgnores(rootpath)
		if err != nil {
			return GlobalHashableInputs{}, err
		}

		f, err := globby.GlobFiles(rootpath.ToStringDuringMigration(), globalFileDependencies, ignores)
		if err != nil {
			return GlobalHashableInputs{}, err
		}

		for _, val := range f {
			globalDeps.Add(val)
		}
	}

	if lockFile == nil {
		// If we don't have lockfile information available, add the specfile and lockfile to global deps
		globalDeps.Add(filepath.Join(rootpath.ToStringDuringMigration(), packageManager.Specfile))
		if rootpath.UntypedJoin(packageManager.Lockfile).Exists() {
			globalDeps.Add(filepath.Join(rootpath.ToStringDuringMigration(), packageManager.Lockfile))
		}
	}

	// No prefix, global deps already have full paths
	globalDepsArray := globalDeps.UnsafeListOfStrings()
	globalDepsPaths := make([]turbopath.AnchoredSystemPath, len(globalDepsArray))
	for i, path := range globalDepsArray {
		fullyQualifiedPath := turbopath.AbsoluteSystemPathFromUpstream(path)
		anchoredPath, err := fullyQualifiedPath.RelativeTo(rootpath)
		if err != nil {
			return GlobalHashableInputs{}, err
		}

		globalDepsPaths[i] = anchoredPath
	}

	globalFileHashMap, err := hashing.GetHashesForFiles(rootpath, globalDepsPaths)
	if err != nil {
		return GlobalHashableInputs{}, fmt.Errorf("error hashing files: %w", err)
	}

	// Make sure we include specified .env files in the file hash.
	// Handled separately because these are not globs!
	if len(dotEnv) > 0 {
		dotEnvObject, err := hashing.GetHashesForExistingFiles(rootpath, dotEnv.ToSystemPathArray())
		if err != nil {
			return GlobalHashableInputs{}, fmt.Errorf("error hashing files: %w", err)
		}

		// Add the dotEnv files into the file hash object.
		for key, value := range dotEnvObject {
			globalFileHashMap[key] = value
		}
	}

	return GlobalHashableInputs{
		globalCacheKey:       _globalCacheKey,
		globalFileHashMap:    globalFileHashMap,
		rootExternalDepsHash: rootPackageJSON.ExternalDepsHash,
		env:                  globalEnv,
		resolvedEnvVars:      globalHashableEnvVars,
		passThroughEnv:       globalPassThroughEnv,
		envMode:              envMode,
		frameworkInference:   frameworkInference,
		dotEnv:               dotEnv,
	}, nil
}
