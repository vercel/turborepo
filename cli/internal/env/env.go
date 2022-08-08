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

// getEnvPairsFromPrefix returns a slice of all key=value pairs that match the given prefix
func getEnvPairsFromPrefix(includePrefix string, excludePrefix string, allEnvVars map[string]string) []string {
	allEnvPairs := []string{}
	for k, v := range allEnvVars {
		if excludePrefix != "" && strings.HasPrefix(k, excludePrefix) {
			continue
		}
		if strings.HasPrefix(k, includePrefix) {
			allEnvPairs = append(allEnvPairs, fmt.Sprintf("%v=%v", k, v))
		}
	}
	return allEnvPairs
}

// getEnvPairsFromPrefixes returns a slice containing key=value pairs
func getEnvPairsFromPrefixes(includePrefixes []string, excludePrefix string, allEnvVars map[string]string) []string {
	allEnvPairs := []string{}
	for _, includePrefix := range includePrefixes {
		hashableFrameworkEnvPairs := getEnvPairsFromPrefix(includePrefix, excludePrefix, allEnvVars)
		allEnvPairs = append(allEnvPairs, hashableFrameworkEnvPairs...)
	}
	return allEnvPairs
}

// GetHashableEnvPairs returns all sorted key=value env var pairs for both frameworks and from envKeys
func GetHashableEnvPairs(envKeys []string, envPrefixes []string) []string {
	allEnvVars := getEnvMap()
	excludePrefix := allEnvVars["TURBO_CI_VENDOR_ENV_KEY"]
	hashableEnvFromKeys := getEnvPairsFromKeys(envKeys, allEnvVars)
	hashableEnvFromPrefixes := getEnvPairsFromPrefixes(envPrefixes, excludePrefix, allEnvVars)

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
