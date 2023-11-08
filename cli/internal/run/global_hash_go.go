//go:build go || !rust
// +build go !rust

package run

import "github.com/vercel/turbo/cli/internal/env"

// `getGlobalHashableEnvVars` calculates env var dependencies
func getGlobalHashableEnvVars(envAtExecutionStart env.EnvironmentVariableMap, globalEnv []string) (env.DetailedMap, error) {
	// Our "inferred" env var maps
	defaultEnvVarMap, err := envAtExecutionStart.FromWildcards(_defaultEnvVars)
	if err != nil {
		return env.DetailedMap{}, err
	}
	userEnvVarSet, err := envAtExecutionStart.FromWildcardsUnresolved(globalEnv)
	if err != nil {
		return env.DetailedMap{}, err
	}

	allEnvVarMap := env.EnvironmentVariableMap{}
	allEnvVarMap.Union(userEnvVarSet.Inclusions)
	allEnvVarMap.Union(defaultEnvVarMap)
	allEnvVarMap.Difference(userEnvVarSet.Exclusions)

	explicitEnvVarMap := env.EnvironmentVariableMap{}
	explicitEnvVarMap.Union(userEnvVarSet.Inclusions)
	explicitEnvVarMap.Difference(userEnvVarSet.Exclusions)

	matchingEnvVarMap := env.EnvironmentVariableMap{}
	matchingEnvVarMap.Union(defaultEnvVarMap)
	matchingEnvVarMap.Difference(userEnvVarSet.Exclusions)

	globalHashableEnvVars := env.DetailedMap{
		All: allEnvVarMap,
		BySource: env.BySource{
			Explicit: explicitEnvVarMap,
			Matching: matchingEnvVarMap,
		},
	}

	return globalHashableEnvVars, nil
}
