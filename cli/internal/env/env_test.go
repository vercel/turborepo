package env

import (
	"reflect"
	"testing"

	"gotest.tools/v3/assert"
)

func TestGetEnvVarsFromWildcards(t *testing.T) {
	tests := []struct {
		name             string
		self             EnvironmentVariableMap
		wildcardPatterns []string
		want             EnvironmentVariableMap
	}{
		{
			name:             "nil wildcard patterns",
			self:             EnvironmentVariableMap{},
			wildcardPatterns: nil,
			want:             nil,
		},
		{
			name:             "empty wildcard patterns",
			self:             EnvironmentVariableMap{},
			wildcardPatterns: []string{},
			want:             EnvironmentVariableMap{},
		},
		{
			name: "leading wildcard",
			self: EnvironmentVariableMap{
				"STATIC":     "VALUE",
				"_STATIC":    "VALUE",
				"FOO_STATIC": "VALUE",
			},
			wildcardPatterns: []string{"*_STATIC"},
			want: EnvironmentVariableMap{
				"_STATIC":    "VALUE",
				"FOO_STATIC": "VALUE",
			},
		},
		{
			name: "trailing wildcard",
			self: EnvironmentVariableMap{
				"STATIC":         "VALUE",
				"STATIC_":        "VALUE",
				"STATIC_TRAILER": "VALUE",
			},
			wildcardPatterns: []string{"STATIC_*"},
			want: EnvironmentVariableMap{
				"STATIC_":        "VALUE",
				"STATIC_TRAILER": "VALUE",
			},
		},
		{
			name: "leading & trailing wildcard",
			self: EnvironmentVariableMap{
				"STATIC":     "VALUE",
				"STATIC_":    "VALUE",
				"_STATIC":    "VALUE",
				"_STATIC_":   "VALUE",
				"_STATIC_B":  "VALUE",
				"A_STATIC_":  "VALUE",
				"A_STATIC_B": "VALUE",
			},
			wildcardPatterns: []string{"*_STATIC_*"},
			want: EnvironmentVariableMap{
				"_STATIC_":   "VALUE",
				"_STATIC_B":  "VALUE",
				"A_STATIC_":  "VALUE",
				"A_STATIC_B": "VALUE",
			},
		},
		{
			name: "adjacent wildcard",
			self: EnvironmentVariableMap{
				"FOO__BAR":   "VALUE",
				"FOO_1_BAR":  "VALUE",
				"FOO_12_BAR": "VALUE",
			},
			wildcardPatterns: []string{"FOO_**_BAR"},
			want: EnvironmentVariableMap{
				"FOO__BAR":   "VALUE",
				"FOO_1_BAR":  "VALUE",
				"FOO_12_BAR": "VALUE",
			},
		},
		{
			name: "literal *",
			self: EnvironmentVariableMap{
				"LITERAL_*": "VALUE",
			},
			wildcardPatterns: []string{"LITERAL_\\*"},
			want: EnvironmentVariableMap{
				"LITERAL_*": "VALUE",
			},
		},
		{
			name: "literal *, then wildcard",
			self: EnvironmentVariableMap{
				"LITERAL_*":          "VALUE",
				"LITERAL_*_ANYTHING": "VALUE",
			},
			wildcardPatterns: []string{"LITERAL_\\**"},
			want: EnvironmentVariableMap{
				"LITERAL_*":          "VALUE",
				"LITERAL_*_ANYTHING": "VALUE",
			},
		},
		// Check ! for exclusion.
		{
			name: "literal leading !",
			self: EnvironmentVariableMap{
				"!LITERAL": "VALUE",
			},
			wildcardPatterns: []string{"\\!LITERAL"},
			want: EnvironmentVariableMap{
				"!LITERAL": "VALUE",
			},
		},
		{
			name: "literal ! anywhere else",
			self: EnvironmentVariableMap{
				"ANYWHERE!ELSE": "VALUE",
			},
			wildcardPatterns: []string{"ANYWHERE!ELSE"},
			want: EnvironmentVariableMap{
				"ANYWHERE!ELSE": "VALUE",
			},
		},
		// The following tests are to confirm exclusion behavior.
		// They're focused on set difference, not wildcard behavior.
		// Wildcard regex construction is identical to inclusions.
		{
			name: "include everything",
			self: EnvironmentVariableMap{
				"ALL":      "VALUE",
				"OF":       "VALUE",
				"THESE":    "VALUE",
				"ARE":      "VALUE",
				"INCLUDED": "VALUE",
			},
			wildcardPatterns: []string{"*"},
			want: EnvironmentVariableMap{
				"ALL":      "VALUE",
				"OF":       "VALUE",
				"THESE":    "VALUE",
				"ARE":      "VALUE",
				"INCLUDED": "VALUE",
			},
		},
		{
			name: "include everything, exclude everything",
			self: EnvironmentVariableMap{
				"ALL":      "VALUE",
				"OF":       "VALUE",
				"THESE":    "VALUE",
				"ARE":      "VALUE",
				"EXCLUDED": "VALUE",
			},
			wildcardPatterns: []string{"*", "!*"},
			want:             EnvironmentVariableMap{},
		},
		{
			name: "include everything, exclude one",
			self: EnvironmentVariableMap{
				"ONE":      "VALUE",
				"OF":       "VALUE",
				"THESE":    "VALUE",
				"IS":       "VALUE",
				"EXCLUDED": "VALUE",
			},
			wildcardPatterns: []string{"*", "!EXCLUDED"},
			want: EnvironmentVariableMap{
				"ONE":   "VALUE",
				"OF":    "VALUE",
				"THESE": "VALUE",
				"IS":    "VALUE",
			},
		},
		{
			name: "include everything, exclude a prefix",
			self: EnvironmentVariableMap{
				"EXCLUDED_SHA":  "VALUE",
				"EXCLUDED_URL":  "VALUE",
				"EXCLUDED_USER": "VALUE",
				"EXCLUDED_PASS": "VALUE",
				"THIS":          "VALUE",
				"IS":            "VALUE",
				"INCLUDED":      "VALUE",
			},
			wildcardPatterns: []string{"*", "!EXCLUDED_*"},
			want: EnvironmentVariableMap{
				"THIS":     "VALUE",
				"IS":       "VALUE",
				"INCLUDED": "VALUE",
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := tt.self.FromWildcards(tt.wildcardPatterns)
			assert.NilError(t, err, "Did not fail regexp compile.")
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GetEnvVarsFromWildcards() = %v, want %v", got, tt.want)
			}
		})
	}
}
