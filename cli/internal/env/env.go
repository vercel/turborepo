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

// EnvironmentVariablePairs is a list of "k=v" strings for env variables and their values
type EnvironmentVariablePairs []string

type wildcardSet struct {
	Inclusions EnvironmentVariableMap
	Exclusions EnvironmentVariableMap
}

func (ws wildcardSet) Resolved() EnvironmentVariableMap {
	output := EnvironmentVariableMap{}
	output.Merge(ws.Inclusions)
	output.Remove(ws.Exclusions)
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

// Merge takes another EnvironmentVariableMap and merges it into the receiver
// It overwrites values if they already exist, but since the source of both will be os.Environ()
// it doesn't matter
func (evm EnvironmentVariableMap) Merge(another EnvironmentVariableMap) {
	for k, v := range another {
		evm[k] = v
	}
}

// Remove takes another EnvironmentVariableMap and removes matching keys from the receiver
func (evm EnvironmentVariableMap) Remove(another EnvironmentVariableMap) {
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

func wildcardToRegexPattern(pattern string) (*string, string) {
	hasWildcard := false
	var regexString []string
	var literalString []string

	var previousIndex int
	var previousRune rune

	for i, char := range pattern {
		if char == wildcard {
			if previousRune == wildcardEscape {
				// Found a literal *

				// Replace the trailing "\*" with just "*" before adding the segment.
				literalString = append(literalString, pattern[previousIndex:i-1]+"*")
				regexString = append(regexString, regexp.QuoteMeta(pattern[previousIndex:i-1]+"*"))
			} else {
				// Found a wildcard
				hasWildcard = true

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
	literalString = append(literalString, pattern[previousIndex:])
	regexString = append(regexString, regexp.QuoteMeta(pattern[previousIndex:]))

	// We need the computed literal env value because FOO= is meaningful.
	var literalValue string
	if !hasWildcard {
		literalValue = strings.Join(literalString, "")
		return &literalValue, strings.Join(regexString, "")
	}

	return nil, strings.Join(regexString, "")
}

// FromWildcards returns an EnvironmentVariableMap after processing wildcards against it.
func (evm EnvironmentVariableMap) fromWildcards(wildcardPatterns []string) (wildcardSet, error) {
	output := wildcardSet{
		Inclusions: EnvironmentVariableMap{},
		Exclusions: EnvironmentVariableMap{},
	}

	includePatterns := make([]string, 0)
	excludePatterns := make([]string, 0)

	for _, wildcardPattern := range wildcardPatterns {
		isExclude := strings.HasPrefix(wildcardPattern, "!")
		isLiteralLeadingExclamation := strings.HasPrefix(wildcardPattern, "\\!")

		if isExclude {
			_, excludePattern := wildcardToRegexPattern(wildcardPattern[1:])
			excludePatterns = append(excludePatterns, excludePattern)
		} else if isLiteralLeadingExclamation {
			includeLiteral, includePattern := wildcardToRegexPattern(wildcardPattern[1:])
			if includeLiteral != nil {
				output.Inclusions[*includeLiteral] = ""
			}
			includePatterns = append(includePatterns, includePattern)
		} else {
			includeLiteral, includePattern := wildcardToRegexPattern(wildcardPattern[0:])
			includePatterns = append(includePatterns, includePattern)
			if includeLiteral != nil {
				output.Inclusions[*includeLiteral] = ""
			}
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

func (evm EnvironmentVariableMap) FromWildcards(wildcardPatterns []string) (EnvironmentVariableMap, error) {
	if wildcardPatterns == nil {
		return nil, nil
	}

	resolvedSet, err := evm.fromWildcards(wildcardPatterns)
	if err != nil {
		return nil, err
	}

	return resolvedSet.Resolved(), nil
}

func (evm EnvironmentVariableMap) FromWildcardsInclusionsOnly(wildcardPatterns []string) (EnvironmentVariableMap, error) {
	if wildcardPatterns == nil {
		return nil, nil
	}

	resolvedSet, err := evm.fromWildcards(wildcardPatterns)
	if err != nil {
		return nil, err
	}

	return resolvedSet.Inclusions, nil
}

func (evm EnvironmentVariableMap) FromWildcardsExclusionsOnly(wildcardPatterns []string) (EnvironmentVariableMap, error) {
	if wildcardPatterns == nil {
		return nil, nil
	}

	resolvedSet, err := evm.fromWildcards(wildcardPatterns)
	if err != nil {
		return nil, err
	}

	return resolvedSet.Exclusions, nil
}

func (evm EnvironmentVariableMap) GetHashableEnvVars(keys []string, matchers []string, envVarContainingExcludePrefix string) (DetailedMap, error) {
	output := DetailedMap{}

	inclusions, err := evm.FromWildcardsInclusionsOnly(keys)
	if err != nil {
		return output, err
	}

	wildcards := []string{}
	wildcards = append(wildcards, matchers...)
	if envVarContainingExcludePrefix != "" && evm[envVarContainingExcludePrefix] != "" {
		wildcards = append(wildcards, "!"+evm[envVarContainingExcludePrefix]+"*")
	}

	matched, err := evm.FromWildcards(wildcards)
	if err != nil {
		return output, err
	}

	all := EnvironmentVariableMap{}
	all.Merge(inclusions)
	all.Merge(matched)

	output.All = all
	output.BySource.Explicit = inclusions
	output.BySource.Matching = matched

	return output, nil
}
