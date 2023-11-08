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
// BySource is used to print out a Dry Run Summary
type DetailedMap struct {
	All      EnvironmentVariableMap
	BySource BySource
}

// EnvironmentVariablePairs is a list of "k=v" strings for env variables and their values
type EnvironmentVariablePairs []string

// WildcardMaps is a pair of EnvironmentVariableMaps.
type WildcardMaps struct {
	Inclusions EnvironmentVariableMap
	Exclusions EnvironmentVariableMap
}

// Resolve collapses a WildcardSet into a single EnvironmentVariableMap.
func (ws WildcardMaps) Resolve() EnvironmentVariableMap {
	output := EnvironmentVariableMap{}
	output.Union(ws.Inclusions)
	output.Difference(ws.Exclusions)
	return output
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

// Union takes another EnvironmentVariableMap and adds it into the receiver
// It overwrites values if they already exist.
func (evm EnvironmentVariableMap) Union(another EnvironmentVariableMap) {
	for k, v := range another {
		evm[k] = v
	}
}

// Difference takes another EnvironmentVariableMap and removes matching keys from the receiver
func (evm EnvironmentVariableMap) Difference(another EnvironmentVariableMap) {
	for k := range another {
		delete(evm, k)
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

const wildcard = '*'
const wildcardEscape = '\\'
const regexWildcardSegment = ".*"

func wildcardToRegexPattern(pattern string) string {
	var regexString []string

	var previousIndex int
	var previousRune rune

	for i, char := range pattern {
		if char == wildcard {
			if previousRune == wildcardEscape {
				// Found a literal *

				// Replace the trailing "\*" with just "*" before adding the segment.
				regexString = append(regexString, regexp.QuoteMeta(pattern[previousIndex:i-1]+"*"))
			} else {
				// Found a wildcard

				// Add in the static segment since the last wildcard. Can be zero length.
				regexString = append(regexString, regexp.QuoteMeta(pattern[previousIndex:i]))

				// Add a dynamic segment if it isn't adjacent to another dynamic segment.
				if regexString[len(regexString)-1] != regexWildcardSegment {
					regexString = append(regexString, regexWildcardSegment)
				}
			}

			// Advance the pointer.
			previousIndex = i + 1
		}
		previousRune = char
	}

	// Add the last static segment. Can be zero length.
	regexString = append(regexString, regexp.QuoteMeta(pattern[previousIndex:]))

	return strings.Join(regexString, "")
}

// fromWildcards returns a wildcardSet after processing wildcards against it.
func (evm EnvironmentVariableMap) fromWildcards(wildcardPatterns []string) (WildcardMaps, error) {
	output := WildcardMaps{
		Inclusions: EnvironmentVariableMap{},
		Exclusions: EnvironmentVariableMap{},
	}

	includePatterns := make([]string, 0)
	excludePatterns := make([]string, 0)

	for _, wildcardPattern := range wildcardPatterns {
		isExclude := strings.HasPrefix(wildcardPattern, "!")
		isLiteralLeadingExclamation := strings.HasPrefix(wildcardPattern, "\\!")

		if isExclude {
			excludePattern := wildcardToRegexPattern(wildcardPattern[1:])
			excludePatterns = append(excludePatterns, excludePattern)
		} else if isLiteralLeadingExclamation {
			includePattern := wildcardToRegexPattern(wildcardPattern[1:])
			includePatterns = append(includePatterns, includePattern)
		} else {
			includePattern := wildcardToRegexPattern(wildcardPattern[0:])
			includePatterns = append(includePatterns, includePattern)
		}
	}

	includeRegexString := "^(" + strings.Join(includePatterns, "|") + ")$"
	excludeRegexString := "^(" + strings.Join(excludePatterns, "|") + ")$"

	includeRegex, err := regexp.Compile(includeRegexString)
	if err != nil {
		return output, err
	}

	excludeRegex, err := regexp.Compile(excludeRegexString)
	if err != nil {
		return output, err
	}

	for envVar, envValue := range evm {
		if len(includePatterns) > 0 && includeRegex.MatchString(envVar) {
			output.Inclusions[envVar] = envValue
		}
		if len(excludePatterns) > 0 && excludeRegex.MatchString(envVar) {
			output.Exclusions[envVar] = envValue
		}
	}

	return output, nil
}

// FromWildcards returns an EnvironmentVariableMap containing the variables
// in the environment which match an array of wildcard patterns.
func (evm EnvironmentVariableMap) FromWildcards(wildcardPatterns []string) (EnvironmentVariableMap, error) {
	if wildcardPatterns == nil {
		return nil, nil
	}

	resolvedSet, err := evm.fromWildcards(wildcardPatterns)
	if err != nil {
		return nil, err
	}

	return resolvedSet.Resolve(), nil
}

// FromWildcardsUnresolved returns a wildcardSet specifying the inclusions and
// exclusions discovered from a set of wildcard patterns. This is used to ensure
// that user exclusions have primacy over inferred inclusions.
func (evm EnvironmentVariableMap) FromWildcardsUnresolved(wildcardPatterns []string) (WildcardMaps, error) {
	if wildcardPatterns == nil {
		return WildcardMaps{}, nil
	}

	return evm.fromWildcards(wildcardPatterns)
}
