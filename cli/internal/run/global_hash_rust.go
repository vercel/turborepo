//go:build rust
// +build rust

package run

import (
	"github.com/vercel/turbo/cli/internal/env"

	"github.com/vercel/turbo/cli/internal/ffi"
)

// `getGlobalHashableEnvVars` calculates env var dependencies
func getGlobalHashableEnvVars(envAtExecutionStart env.EnvironmentVariableMap, globalEnv []string) (env.DetailedMap, error) {
	respDetailedMap, err := ffi.GetGlobalHashableEnvVars(envAtExecutionStart, globalEnv)
	if err != nil {
		return env.DetailedMap{}, err
	}

	// We set explicit and matching to empty maps if they are nil
	// to preserve existing behavior from the Go code
	explicit := respDetailedMap.GetBySource().GetExplicit()
	if explicit == nil {
		explicit = make(map[string]string)
	}

	matching := respDetailedMap.GetBySource().GetMatching()
	if matching == nil {
		matching = make(map[string]string)
	}
	detailedMap := env.DetailedMap{
		All: respDetailedMap.GetAll(),
		BySource: env.BySource{
			Explicit: explicit,
			Matching: matching,
		},
	}
	return detailedMap, nil
}
