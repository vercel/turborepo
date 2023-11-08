package run

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/env"
)

func TestGetGlobalHashableEnvVars(t *testing.T) {
	testCases := []struct {
		name                string
		envAtExecutionStart env.EnvironmentVariableMap
		globalEnv           []string
		expectedMap         env.DetailedMap
	}{
		{
			name: "has default env var",
			envAtExecutionStart: env.EnvironmentVariableMap{
				"VERCEL_ANALYTICS_ID": "123",
			},
			globalEnv: []string{},
			expectedMap: env.DetailedMap{
				All: map[string]string{
					"VERCEL_ANALYTICS_ID": "123",
				},
				BySource: env.BySource{
					Matching: map[string]string{
						"VERCEL_ANALYTICS_ID": "123",
					},
					Explicit: map[string]string{},
				},
			},
		},
		{
			name: "has global env wildcard",
			envAtExecutionStart: env.EnvironmentVariableMap{
				"FOO_BAR": "123",
			},
			globalEnv: []string{"FOO*"},
			expectedMap: env.DetailedMap{
				All: map[string]string{
					"FOO_BAR": "123",
				},
				BySource: env.BySource{
					Matching: map[string]string{},
					Explicit: map[string]string{
						"FOO_BAR": "123",
					},
				},
			},
		},
		{
			name: "has global env wildcard but also excluded",
			envAtExecutionStart: env.EnvironmentVariableMap{
				"FOO_BAR": "123",
			},
			globalEnv: []string{"FOO*", "!FOO_BAR"},
			expectedMap: env.DetailedMap{
				BySource: env.BySource{
					Matching: map[string]string{},
					Explicit: map[string]string{},
				},
			},
		},
	}

	for _, testCase := range testCases {
		t.Run(testCase.name, func(t *testing.T) {
			result, err := getGlobalHashableEnvVars(testCase.envAtExecutionStart, testCase.globalEnv)
			assert.NoError(t, err)
			assert.Equal(t, testCase.expectedMap, result)
		})
	}
}
