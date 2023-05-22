package env

import (
	"reflect"
	"testing"
)

func TestGetEnvVarsFromWildcards(t *testing.T) {
	tests := []struct {
		name             string
		self             EnvironmentVariableMap
		wildcardPatterns []string
		want             EnvironmentVariableMap
		wantErr          bool
	}{
		{
			name:             "nil wildcard patterns",
			self:             EnvironmentVariableMap{},
			wildcardPatterns: nil,
			want:             nil,
			wantErr:          false,
		},
		{
			name:             "empty wildcard patterns",
			self:             EnvironmentVariableMap{},
			wildcardPatterns: []string{},
			want:             EnvironmentVariableMap{},
			wantErr:          false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr: false,
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
			wantErr:          false,
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
			wantErr: false,
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
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := tt.self.FromWildcards(tt.wildcardPatterns)
			if (err != nil) != tt.wantErr {
				t.Errorf("GetEnvVarsFromWildcards() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GetEnvVarsFromWildcards() = %v, want %v", got, tt.want)
			}
		})
	}
}
