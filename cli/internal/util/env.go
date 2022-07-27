package util

import (
	"fmt"
	"os"
	"sort"
	"strings"
)

// Prefixes for common framework variables that we always include
var envVarPrefixes = []string{
	"GATSBY_",
	"NEXT_PUBLIC_",
	"NUXT_ENV_",
	"PUBLIC_",
	"REACT_APP_",
	"REDWOOD_ENV_",
	"SANITY_STUDIO_",
	"VITE_",
	"VUE_APP_",
}

// getEnvPairsFromKeys returns a slice of key=value pairs for all env var keys specified in envKeys
func getEnvPairsFromKeys(envKeys []string, allEnvVars []string) []string {
	hashableConfigEnvPairs := []string{}
	for _, envVar := range envKeys {
		hashableConfigEnvPairs = append(hashableConfigEnvPairs, fmt.Sprintf("%v=%v", envVar, os.Getenv(envVar)))
	}

	return hashableConfigEnvPairs
}

// getFrameworkEnvPairs returns a slice of all key=value pairs that match the given prefix
func getFrameworkEnvPairs(prefix string, allEnvVars []string) []string {
	hashableFrameworkEnvPairs := []string{}
	for _, pair := range allEnvVars {
		if strings.HasPrefix(pair, prefix) {
			hashableFrameworkEnvPairs = append(hashableFrameworkEnvPairs, pair)
		}
	}
	return hashableFrameworkEnvPairs
}

// getEnvPairsFromPrefixes returns a slice containing key=value pairs for all frameworks
func getEnvPairsFromPrefixes(prefixes []string, allEnvVars []string) []string {
	allHashableFrameworkEnvPairs := []string{}
	for _, frameworkEnvPrefix := range envVarPrefixes {
		hashableFrameworkEnvPairs := getFrameworkEnvPairs(frameworkEnvPrefix, allEnvVars)
		allHashableFrameworkEnvPairs = append(allHashableFrameworkEnvPairs, hashableFrameworkEnvPairs...)

	}
	return allHashableFrameworkEnvPairs
}

// GetHashableEnvPairs returns all sorted key=value env var pairs for both frameworks and from envKeys
func GetHashableEnvPairs(envKeys []string) []string {
	allEnvVars := os.Environ()
	hashableEnvFromKeys := getEnvPairsFromKeys(envKeys, allEnvVars)
	hashableEnvFromPrefixes := getEnvPairsFromPrefixes(envVarPrefixes, allEnvVars)

	// convert to set to eliminate duplicates, then cast back to slice to sort for stable hashing
	allHashableEnvPairs := SetFromStrings(append(hashableEnvFromKeys, hashableEnvFromPrefixes...)).UnsafeListOfStrings()
	sort.Strings(allHashableEnvPairs)
	return allHashableEnvPairs
}
