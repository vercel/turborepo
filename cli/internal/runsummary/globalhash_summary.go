package runsummary

import (
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GlobalHashSummary contains the pieces of data that impacted the global hash (then then impacted the task hash)
type GlobalHashSummary struct {
	GlobalCacheKey       string                                `json:"rootKey"`
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string `json:"files"`
	RootExternalDepsHash string                                `json:"hashOfExternalDependencies"`
	Env                  []string                              `json:"globalEnv,omitempty"`
	PassThroughEnv       []string                              `json:"globalPassThroughEnv"`
	DotEnv               turbopath.AnchoredUnixPathArray       `json:"globalDotEnv"`

	// This is a private field because and not in JSON, because we'll add it to each task
	resolvedEnvVars            env.EnvironmentVariablePairs
	resolvedPassThroughEnvVars env.EnvironmentVariablePairs
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
		Env:                  globalEnv,
		PassThroughEnv:       globalPassThroughEnv,
		DotEnv:               globalDotEnv,

		resolvedEnvVars:            resolvedEnvVars.All.ToSecretHashable(),
		resolvedPassThroughEnvVars: resolvedPassThroughEnvVars.ToSecretHashable(),
	}
}
