package runsummary

import (
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GlobalEnvConfiguration contains the environment variable inputs for the global hash
type GlobalEnvConfiguration struct {
	Env            []string `json:"env"`
	PassThroughEnv []string `json:"passThroughEnv"`
}

// GlobalEnvVarSummary contains the environment variables that impacted the global hash
type GlobalEnvVarSummary struct {
	Specified GlobalEnvConfiguration `json:"specified"`

	Configured  env.EnvironmentVariablePairs `json:"configured"`
	Inferred    env.EnvironmentVariablePairs `json:"inferred"`
	PassThrough env.EnvironmentVariablePairs `json:"passthrough"`
}

// GlobalHashSummary contains the pieces of data that impacted the global hash (then then impacted the task hash)
type GlobalHashSummary struct {
	GlobalCacheKey       string                                `json:"rootKey"`
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string `json:"files"`
	RootExternalDepsHash string                                `json:"hashOfExternalDependencies"`
	DotEnv               turbopath.AnchoredUnixPathArray       `json:"globalDotEnv"`
	EnvVars              GlobalEnvVarSummary                   `json:"environmentVariables"`
}

// NewGlobalHashSummary creates a GlobalHashSummary struct from a set of fields.
func NewGlobalHashSummary(
	globalCacheKey string,
	fileHashMap map[turbopath.AnchoredUnixPath]string,
	rootExternalDepsHash string,
	globalEnv []string,
	globalPassThroughEnv []string,
	globalDotEnv turbopath.AnchoredUnixPathArray,
	resolvedEnvVars env.DetailedMap,
	resolvedPassThroughEnvVars env.EnvironmentVariableMap,
) *GlobalHashSummary {
	return &GlobalHashSummary{
		GlobalCacheKey:       globalCacheKey,
		GlobalFileHashMap:    fileHashMap,
		RootExternalDepsHash: rootExternalDepsHash,

		EnvVars: GlobalEnvVarSummary{
			Specified: GlobalEnvConfiguration{
				Env:            globalEnv,
				PassThroughEnv: globalPassThroughEnv,
			},
			Configured:  resolvedEnvVars.BySource.Explicit.ToSecretHashable(),
			Inferred:    resolvedEnvVars.BySource.Matching.ToSecretHashable(),
			PassThrough: resolvedPassThroughEnvVars.ToSecretHashable(),
		},

		DotEnv: globalDotEnv,
	}
}
