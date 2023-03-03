package runsummary

import (
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

// GlobalHashSummary contains the pieces of data that impacted the global hash (then then impacted the task hash)
type GlobalHashSummary struct {
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string `json:"globalFileHashMap"`
	RootExternalDepsHash string                                `json:"rootExternalDepsHash"`
	GlobalCacheKey       string                                `json:"globalCacheKey"`
	Pipeline             fs.PristinePipeline                   `json:"pipeline"`
}

// NewGlobalHashSummary creates a GlobalHashSummary struct from a set of fields.
func NewGlobalHashSummary(globalFileHashMap map[turbopath.AnchoredUnixPath]string, rootExternalDepsHash string, hashedSortedEnvPairs []string, globalCacheKey string, pipeline fs.PristinePipeline) *GlobalHashSummary {
	// TODO(mehulkar): Add hashedSortedEnvPairs in here, but redact the values
	return &GlobalHashSummary{
		GlobalFileHashMap:    globalFileHashMap,
		RootExternalDepsHash: rootExternalDepsHash,
		GlobalCacheKey:       globalCacheKey,
		Pipeline:             pipeline,
	}
}
