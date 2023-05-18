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

	// This is a private field because and not in JSON, because we'll add it to each task
	envVars            env.EnvironmentVariablePairs
	passthroughEnvVars env.EnvironmentVariablePairs
}

// NewGlobalHashSummary creates a GlobalHashSummary struct from a set of fields.
func NewGlobalHashSummary(
	fileHashMap map[turbopath.AnchoredUnixPath]string,
	rootExternalDepsHash string,
	envVars env.DetailedMap,
	passthroughEnvVars env.EnvironmentVariableMap,
	globalCacheKey string,
) *GlobalHashSummary {
	return &GlobalHashSummary{
		envVars:              envVars.All.ToSecretHashable(),
		passthroughEnvVars:   passthroughEnvVars.ToSecretHashable(),
		GlobalFileHashMap:    fileHashMap,
		RootExternalDepsHash: rootExternalDepsHash,
		GlobalCacheKey:       globalCacheKey,
	}
}
