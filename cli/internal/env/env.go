package env

import (
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/vercel/turborepo/cli/internal/util"
)

func getEnvMap() map[string]string {
	envMap := make(map[string]string)
	for _, envVar := range os.Environ() {
		if i := strings.Index(envVar, "="); i >= 0 {
			parts := strings.SplitN(envVar, "=", 2)
			envMap[parts[0]] = strings.Join(parts[1:], "")
		}
	}
	return envMap
}

// getEnvPairsFromKeys returns a slice of key=value pairs for all env var keys specified in envKeys
func getEnvPairsFromKeys(envKeys []string, allEnvVars map[string]string) []string {
	hashableConfigEnvPairs := []string{}
	for _, envVar := range envKeys {
		hashableConfigEnvPairs = append(hashableConfigEnvPairs, fmt.Sprintf("%v=%v", envVar, allEnvVars[envVar]))
	}

	return hashableConfigEnvPairs
}

// getFrameworkEnvPairs returns a slice of all key=value pairs that match the given prefix
func getEnvPairsFromPrefix(prefix string, allEnvVars map[string]string) []string {
	hashableFrameworkEnvPairs := []string{}
	for k, v := range allEnvVars {
		if strings.HasPrefix(k, prefix) {
			hashableFrameworkEnvPairs = append(hashableFrameworkEnvPairs, fmt.Sprintf("%v=%v", k, v))
		}
	}
	return hashableFrameworkEnvPairs
}

// getEnvPairsFromPrefixes returns a slice containing key=value pairs for all frameworks
func getEnvPairsFromPrefixes(prefixes []string, allEnvVars map[string]string) []string {
	allHashableFrameworkEnvPairs := []string{}
	for _, frameworkEnvPrefix := range prefixes {
		hashableFrameworkEnvPairs := getEnvPairsFromPrefix(frameworkEnvPrefix, allEnvVars)
		allHashableFrameworkEnvPairs = append(allHashableFrameworkEnvPairs, hashableFrameworkEnvPairs...)

	}
	return allHashableFrameworkEnvPairs
}

// GetHashableEnvPairs returns all sorted key=value env var pairs for both frameworks and from envKeys
func GetHashableEnvPairs(envKeys []string, envPrefixes []string) []string {
	allEnvVars := getEnvMap()
	hashableEnvFromKeys := getEnvPairsFromKeys(envKeys, allEnvVars)
	hashableEnvFromPrefixes := getEnvPairsFromPrefixes(envPrefixes, allEnvVars)

	// convert to set to eliminate duplicates, then cast back to slice to sort for stable hashing
	uniqueHashableEnvPairs := make(util.Set, len(hashableEnvFromKeys)+len(hashableEnvFromPrefixes))
	for _, pair := range hashableEnvFromKeys {
		uniqueHashableEnvPairs.Add(pair)
	}
	for _, pair := range hashableEnvFromPrefixes {
		uniqueHashableEnvPairs.Add(pair)
	}

	allHashableEnvPairs := uniqueHashableEnvPairs.UnsafeListOfStrings()
	sort.Strings(allHashableEnvPairs)
	return allHashableEnvPairs
}
