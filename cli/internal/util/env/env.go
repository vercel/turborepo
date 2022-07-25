package env

import (
	"fmt"
	"os"
	"sort"
	"strings"
)

// Prefixes for common framework variables that we always include
var frameworkPublicEnvPrefix = []string{
	"NEXT_PUBLIC_",
	"NUXT_ENV_",
	"REACT_APP_",
	"GATSBY_",
	"PUBLIC_",
	"VUE_APP_",
	"VITE_",
	"REDWOOD_ENV_",
	"SANITY_STUDIO_",
}

// GetConfigEnvPairs returns a slice of key=value pairs for all env var keys specified in envKeys
func GetConfigEnvPairs(envKeys []string) []string {
	hashableConfigEnvPairs := []string{}
	for _, envVar := range envKeys {
		hashableConfigEnvPairs = append(hashableConfigEnvPairs, fmt.Sprintf("%v=%v", envVar, os.Getenv(envVar)))
	}

	return hashableConfigEnvPairs
}

// getFrameworkEnvPairs returns a slice of all key=value pairs that match the given frameworkEnvPrefix
func getFrameworkEnvPairs(frameworkEnvPrefix string, allEnvVars []string) []string {
	hashableFrameworkEnvPairs := []string{}
	for _, pair := range os.Environ() {
		if strings.HasPrefix(pair, frameworkEnvPrefix) {
			hashableFrameworkEnvPairs = append(hashableFrameworkEnvPairs, pair)
		}
	}
	return hashableFrameworkEnvPairs
}

// GetAllFrameworkEnvPairs returns a slice containing key=value pairs for all frameworks
func GetAllFrameworkEnvPairs() []string {
	allEnvVars := os.Environ()
	allHashableFrameworkEnvPairs := []string{}
	for _, frameworkEnvPrefix := range frameworkPublicEnvPrefix {
		hashableFrameworkEnvPairs := getFrameworkEnvPairs(frameworkEnvPrefix, allEnvVars)
		allHashableFrameworkEnvPairs = append(allHashableFrameworkEnvPairs, hashableFrameworkEnvPairs...)

	}
	return allHashableFrameworkEnvPairs
}

// GetHashableEnvPairs returns all sorted key=value env var pairs for both frameworks and from envKeys
func GetHashableEnvPairs(envKeys []string) []string {
	hashableConfigEnvPairs := GetConfigEnvPairs(envKeys)
	hashableFrameworkEnvPairs := GetAllFrameworkEnvPairs()

	allHashableEnvPairs := append(hashableConfigEnvPairs, hashableFrameworkEnvPairs...)
	sort.Strings(allHashableEnvPairs)
	return allHashableEnvPairs
}
