package runsummary

import (
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GlobalHashSummary contains the pieces of data that impacted the global hash (then then impacted the task hash)
type GlobalHashSummary struct {
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string `json:"globalFileHashMap"`
	RootExternalDepsHash string                                `json:"rootExternalDepsHash"`
	GlobalCacheKey       string                                `json:"globalCacheKey"`
	Pipeline             fs.PristinePipeline                   `json:"pipeline"`
	EnvVars              env.EnvironmentVariablePairs          `json:"-"`
}

// NewGlobalHashSummary creates a GlobalHashSummary struct from a set of fields.
func NewGlobalHashSummary(
	fileHashMap map[turbopath.AnchoredUnixPath]string,
	rootExternalDepsHash string,
	envVars env.DetailedMap,
	globalCacheKey string,
	pipeline fs.PristinePipeline,
) *GlobalHashSummary {
	return &GlobalHashSummary{
		EnvVars:              envVars.All.ToSecretHashable(),
		GlobalFileHashMap:    fileHashMap,
		RootExternalDepsHash: rootExternalDepsHash,
		GlobalCacheKey:       globalCacheKey,
		Pipeline:             pipeline,
	}
}
