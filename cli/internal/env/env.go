package env

import (
	"crypto/sha256"
	"fmt"
	"os"
	"regexp"
	"sort"
	"strings"
)

// EnvironmentVariableMap is a map of env variables and their values
type EnvironmentVariableMap map[string]string

// BySource contains a map of environment variables broken down by the source
type BySource struct {
	Explicit EnvironmentVariableMap
	Matching EnvironmentVariableMap
}

// DetailedMap contains the composite and the detailed maps of environment variables
// All is used as a taskhash input (taskhash.CalculateTaskHash)
// BySoure is used to print out a Dry Run Summary
type DetailedMap struct {
	All      EnvironmentVariableMap
	BySource BySource
}

// Merge takes another EnvironmentVariableMap and merges it into the receiver
// It overwrites values if they already exist, but since the source of both will be os.Environ()
// it doesn't matter
func (evm EnvironmentVariableMap) Merge(another EnvironmentVariableMap) {
	for k, v := range another {
		evm[k] = v
	}
}

// Add creates one new environment variable.
func (evm EnvironmentVariableMap) Add(key string, value string) {
	evm[key] = value
}

// Names returns a sorted list of env var names for the EnvironmentVariableMap
func (evm EnvironmentVariableMap) Names() []string {
	names := []string{}
	for k := range evm {
		names = append(names, k)
	}
	sort.Strings(names)
	return names
}

// EnvironmentVariablePairs is a list of "k=v" strings for env variables and their values
type EnvironmentVariablePairs []string

// mapToPair returns a deterministically sorted set of EnvironmentVariablePairs from an EnvironmentVariableMap
// It takes a transformer value to operate on each key-value pair and return a string
func (evm EnvironmentVariableMap) mapToPair(transformer func(k string, v string) string) EnvironmentVariablePairs {
	if evm == nil {
		return nil
	}

	// convert to set to eliminate duplicates
	pairs := make([]string, 0, len(evm))
	for k, v := range evm {
		paired := transformer(k, v)
		pairs = append(pairs, paired)
	}

	// sort it so it's deterministic
	sort.Strings(pairs)

	return pairs
}

// ToSecretHashable returns a deterministically sorted set of EnvironmentVariablePairs from an EnvironmentVariableMap
// This is the value used to print out the task hash input, so the values are cryptographically hashed
func (evm EnvironmentVariableMap) ToSecretHashable() EnvironmentVariablePairs {
	return evm.mapToPair(func(k, v string) string {
		if v != "" {
			hashedValue := sha256.Sum256([]byte(v))
			return fmt.Sprintf("%v=%x", k, hashedValue)
		}

		return fmt.Sprintf("%v=%s", k, "")
	})
}

// ToHashable returns a deterministically sorted set of EnvironmentVariablePairs from an EnvironmentVariableMap
// This is the value that is used upstream as a task hash input, so we need it to be deterministic
func (evm EnvironmentVariableMap) ToHashable() EnvironmentVariablePairs {
	return evm.mapToPair(func(k, v string) string {
		return fmt.Sprintf("%v=%v", k, v)
	})
}

// GetEnvMap returns a map of env vars and their values from os.Environ
func GetEnvMap() EnvironmentVariableMap {
	envMap := make(map[string]string)
	for _, envVar := range os.Environ() {
		if i := strings.Index(envVar, "="); i >= 0 {
			parts := strings.SplitN(envVar, "=", 2)
			envMap[parts[0]] = strings.Join(parts[1:], "")
		}
	}
	return envMap
}

// FromKeys returns a map of env vars and their values from a given set of env var names
func FromKeys(all EnvironmentVariableMap, keys []string) EnvironmentVariableMap {
	output := EnvironmentVariableMap{}
	for _, key := range keys {
		output[key] = all[key]
	}

	return output
}

func fromMatching(all EnvironmentVariableMap, keyMatchers []string, shouldExclude func(k, v string) bool) (EnvironmentVariableMap, error) {
	output := EnvironmentVariableMap{}
	compileFailures := []string{}

	for _, keyMatcher := range keyMatchers {
		rex, err := regexp.Compile(keyMatcher)
		if err != nil {
			compileFailures = append(compileFailures, keyMatcher)
			continue
		}

		for k, v := range all {
			// we can skip keys based on a shouldExclude function passed in.
			if shouldExclude(k, v) {
				continue
			}

			if rex.Match([]byte(k)) {
				output[k] = v
			}
		}
	}

	if len(compileFailures) > 0 {
		return nil, fmt.Errorf("The following env prefixes failed to compile to regex: %s", strings.Join(compileFailures, ", "))
	}

	return output, nil
}

const wildcard = '*'
const wildcardEscape = '\\'
const regexWildcardSegment = ".*"

func wildcardToRegexPattern(pattern string) string {
	var segments []string

	var previousIndex int
	var previousRune rune

	for i, char := range pattern {
		if char == wildcard {
			if previousRune == wildcardEscape {
				// Found a literal *

				// Replace the trailing "\*" with just "*" before adding the segment.
				segments = append(segments, regexp.QuoteMeta(pattern[previousIndex:i-1]+"*"))
			} else {
				// Found a wildcard

				// Add in the static segment since the last wildcard. Can be zero length.
				segments = append(segments, regexp.QuoteMeta(pattern[previousIndex:i]))

				// Add a dynamic segment if it isn't adjacent to another dynamic segment.
				if segments[len(segments)-1] != regexWildcardSegment {
					segments = append(segments, regexWildcardSegment)
				}
			}

			// Advance the pointer.
			previousIndex = i + 1
		}
		previousRune = char
	}

	// Add the last static segment. Can be zero length.
	segments = append(segments, regexp.QuoteMeta(pattern[previousIndex:]))

	return strings.Join(segments, "")
}

// FromWildcards returns an EnvironmentVariableMap after processing wildcards against it.
func (evm EnvironmentVariableMap) FromWildcards(wildcardPatterns []string) (EnvironmentVariableMap, error) {
	includePatterns := make([]string, 0)
	excludePatterns := make([]string, 0)

	for _, wildcardPattern := range wildcardPatterns {
		isExclude := strings.HasPrefix(wildcardPattern, "!")
		isLiteralLeadingExclamation := strings.HasPrefix(wildcardPattern, "\\!")

		if isExclude {
			excludePatterns = append(excludePatterns, wildcardToRegexPattern(wildcardPattern[1:]))
		} else if isLiteralLeadingExclamation {
			includePatterns = append(includePatterns, wildcardToRegexPattern(wildcardPattern[1:]))
		} else {
			includePatterns = append(includePatterns, wildcardToRegexPattern(wildcardPattern[0:]))
		}
	}

	includeRegexString := "^(" + strings.Join(includePatterns, "|") + ")$"
	excludeRegexString := "^(" + strings.Join(excludePatterns, "|") + ")$"

	includeRegex, err := regexp.Compile(includeRegexString)
	if err != nil {
		return nil, err
	}

	excludeRegex, err := regexp.Compile(excludeRegexString)
	if err != nil {
		return nil, err
	}

	output := EnvironmentVariableMap{}
	for envVar, envValue := range evm {
		if len(includePatterns) > 0 && includeRegex.MatchString(envVar) {
			output[envVar] = envValue
		}
		if len(excludePatterns) > 0 && excludeRegex.MatchString(envVar) {
			delete(output, envVar)
		}
	}

	return output, nil
}

// GetHashableEnvVars returns all sorted key=value env var pairs for both frameworks and from envKeys
func GetHashableEnvVars(keys []string, matchers []string, envVarContainingExcludePrefix string) (DetailedMap, error) {
	all := GetEnvMap()

	detailedMap := DetailedMap{
		All:      EnvironmentVariableMap{},
		BySource: BySource{},
	}

	detailedMap.BySource.Explicit = FromKeys(all, keys)
	detailedMap.All.Merge(detailedMap.BySource.Explicit)

	// Create an excluder function to pass to matcher.
	// We only do this when an envVarContainingExcludePrefix is passed.
	// This isn't the greatest design, but we need this to be optional
	shouldExclude := func(k, v string) bool {
		return false
	}
	if envVarContainingExcludePrefix != "" {
		excludedKeyName := all[envVarContainingExcludePrefix]
		shouldExclude = func(k, v string) bool {
			return excludedKeyName != "" && strings.HasPrefix(k, excludedKeyName)
		}
	}

	matchedEnvVars, err := fromMatching(all, matchers, shouldExclude)

	if err != nil {
		return DetailedMap{}, err
	}

	detailedMap.BySource.Matching = matchedEnvVars
	detailedMap.All.Merge(detailedMap.BySource.Matching)
	return detailedMap, nil
}
