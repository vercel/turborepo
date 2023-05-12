package env

import (
	"os"
	"reflect"
	"strings"
	"testing"
)

func setEnvs(envVars []string) {
	for _, envVar := range envVars {
		parts := strings.SplitN(envVar, "=", 2)
		err := os.Setenv(parts[0], strings.Join(parts[1:], ""))
		if err != nil {
			panic(err)
		}
	}
}

// Prefixes for common framework variables that we always include
var _envVarPrefixes = []string{
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

func TestGetHashableEnvVars(t *testing.T) {
	type args struct {
		envKeys     []string
		envPrefixes []string
	}
	tests := []struct {
		env  []string
		name string
		args args
		want EnvironmentVariablePairs
	}{
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, no env values",
			args: args{
				envKeys:     []string{"myval"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"myval="},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, one env value",
			args: args{
				envKeys:     []string{"lowercase"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"lowercase=stillcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "lowercase=notcool"},
			name: "no framework env vars, duplicate env value",
			args: args{
				envKeys:     []string{"lowercase"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"lowercase=notcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, multiple env values",
			args: args{
				envKeys:     []string{"lowercase", "MY_TEST_VAR"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"MY_TEST_VAR=cool", "lowercase=stillcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers", "NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "one framework env var, multiple env values",
			args: args{
				envKeys:     []string{"lowercase", "MY_TEST_VAR"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"MY_TEST_VAR=cool", "NEXT_PUBLIC_MY_COOL_VAR=cool", "lowercase=stillcool"},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "duplicate framework env var and env values",
			args: args{
				envKeys:     []string{"NEXT_PUBLIC_MY_COOL_VAR"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
		},
		{
			env:  []string{"a=1", "b=2", "c=3", "PUBLIC_myvar=4"},
			name: "sorts correctly",
			args: args{
				envKeys:     []string{"a", "b", "c"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"PUBLIC_myvar=4", "a=1", "b=2", "c=3"},
		},
		{
			env:  []string{"a=1=2", "NEXT_PUBLIC_VALUE_TEST=do=not=do=this"},
			name: "parses env values correctly",
			args: args{
				envKeys:     []string{"a"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_VALUE_TEST=do=not=do=this", "a=1=2"},
		},
		{
			env:  []string{"a=1", "NEXT_PUBLIC_=weird"},
			name: "parses prefix with no ending",
			args: args{
				envKeys:     []string{"a"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_=weird", "a=1"},
		},
		{
			env:  []string{"NEXT_PUBLIC_EMOJI=😋"},
			name: "parses unicode env value",
			args: args{
				envKeys:     []string{},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_EMOJI=😋"},
		},
		{
			env:  []string{"zero=0", "null=null", "nil=nil"},
			name: "parses corner case env values",
			args: args{
				envKeys:     []string{"zero", "null", "nil"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"nil=nil", "null=null", "zero=0"},
		},
		{
			env: []string{"GATSBY_custom=GATSBY",
				"NEXT_PUBLIC_custom=NEXT_PUBLIC",
				"NUXT_ENV_custom=NUXT_ENV",
				"PUBLIC_custom=PUBLIC",
				"REACT_APP_custom=REACT_APP",
				"REDWOOD_ENV_custom=REDWOOD_ENV",
				"SANITY_STUDIO_custom=SANITY_STUDIO",
				"VITE_custom=VITE",
				"VUE_APP_custom=VUE_APP"},
			name: "all framework vars with no env keys",
			args: args{
				envKeys:     []string{},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"GATSBY_custom=GATSBY",
				"NEXT_PUBLIC_custom=NEXT_PUBLIC",
				"NUXT_ENV_custom=NUXT_ENV",
				"PUBLIC_custom=PUBLIC",
				"REACT_APP_custom=REACT_APP",
				"REDWOOD_ENV_custom=REDWOOD_ENV",
				"SANITY_STUDIO_custom=SANITY_STUDIO",
				"VITE_custom=VITE",
				"VUE_APP_custom=VUE_APP"},
		},
		{
			env: []string{"GATSBY_custom=GATSBY",
				"NEXT_PUBLIC_custom=NEXT_PUBLIC",
				"NUXT_ENV_custom=NUXT_ENV",
				"PUBLIC_custom=PUBLIC",
				"REACT_APP_custom=REACT_APP",
				"REDWOOD_ENV_custom=REDWOOD_ENV",
				"SANITY_STUDIO_custom=SANITY_STUDIO",
				"VITE_custom=VITE",
				"CUSTOM=cool",
				"ANOTHER=neat",
				"FINAL=great",
				"VITE_custom=VITE",
				"VUE_APP_custom=VUE_APP"},
			name: "all framework vars with env keys",
			args: args{
				envKeys:     []string{"FINAL", "CUSTOM", "ANOTHER"},
				envPrefixes: _envVarPrefixes,
			},
			want: EnvironmentVariablePairs{"ANOTHER=neat", "CUSTOM=cool", "FINAL=great", "GATSBY_custom=GATSBY",
				"NEXT_PUBLIC_custom=NEXT_PUBLIC",
				"NUXT_ENV_custom=NUXT_ENV",
				"PUBLIC_custom=PUBLIC",
				"REACT_APP_custom=REACT_APP",
				"REDWOOD_ENV_custom=REDWOOD_ENV",
				"SANITY_STUDIO_custom=SANITY_STUDIO",
				"VITE_custom=VITE",
				"VUE_APP_custom=VUE_APP"},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "No framework detected, has framework env vars",
			args: args{
				envKeys:     []string{},
				envPrefixes: []string{},
			},
			want: EnvironmentVariablePairs{},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "Framework detected, has framework env vars",
			args: args{
				envKeys:     []string{},
				envPrefixes: []string{"NEXT_PUBLIC_"},
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_COOL_VAR=cool", "MANUAL=true"},
			name: "Framework detected, has framework env vars, and manually specified key",
			args: args{
				envKeys:     []string{"MANUAL"},
				envPrefixes: []string{"NEXT_PUBLIC_"},
			},
			want: EnvironmentVariablePairs{"MANUAL=true", "NEXT_PUBLIC_MY_COOL_VAR=cool"},
		},
		{
			env:  []string{"MANUAL=true"},
			name: "Framework not detected, has no framework env vars, and manually specified key",
			args: args{
				envKeys:     []string{"MANUAL"},
				envPrefixes: []string{},
			},
			want: EnvironmentVariablePairs{"MANUAL=true"},
		},
		{
			env:  []string{"NEXT_PUBLIC_VERCEL_ENV=true", "MANUAL=true", "MANUAL_VERCEL_ENV=true", "TURBO_CI_VENDOR_ENV_KEY=NEXT_PUBLIC_VERCEL_"},
			name: "$TURBO_CI_VENDOR_ENV_KEY excludes automatically added env vars",
			args: args{
				envKeys:     []string{"MANUAL"},
				envPrefixes: []string{"NEXT_PUBLIC_"},
			},
			want: EnvironmentVariablePairs{"MANUAL=true"},
		},
		{
			env:  []string{"TURBO_ENV=true", "MANUAL=true", "TURBOREPO=true", "TURBO_CI_VENDOR_ENV_KEY=TURBO_"},
			name: "$TURBO_CI_VENDOR_ENV_KEY excludes automatically added env vars",
			args: args{
				envKeys:     []string{},
				envPrefixes: []string{"TURBO"},
			},
			want: EnvironmentVariablePairs{"TURBOREPO=true"},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_VERCEL_URL=me.vercel.com", "TURBOREPO=true", "TURBO_CI_VENDOR_ENV_KEY=NEXT_PUBLIC_VERCEL_"},
			name: "$TURBO_CI_VENDOR_ENV_KEY excludes automatically added env vars",
			args: args{
				envKeys:     []string{"TURBOREPO"},
				envPrefixes: []string{"NEXT_PUBLIC"},
			},
			want: EnvironmentVariablePairs{"NEXT_PUBLIC_MY_VERCEL_URL=me.vercel.com", "TURBOREPO=true"},
		},
		{
			env:  []string{"TURBO_CI_VENDOR_ENV_KEY_VAL=true", "TURBO_CI_VENDOR_ENV_KEY=TURBO_CI_VENDOR_ENV_KEY"},
			name: "$TURBO_CI_VENDOR_ENV_KEY should not exclude itself",
			args: args{
				envKeys:     []string{},
				envPrefixes: []string{"TURBO_"},
			},
			want: EnvironmentVariablePairs{},
		},
		{
			env:  []string{"NEXT_PUBLIC_VERCEL_ENV=true", "MANUAL=true", "TURBO_CI_VENDOR_ENV_KEY=_VERCEL_"},
			name: "blocked env var is allowed if manually specified",
			args: args{
				envKeys:     []string{"NEXT_PUBLIC_VERCEL_ENV", "MANUAL"},
				envPrefixes: []string{"NEXT_PUBLIC_"},
			},
			want: EnvironmentVariablePairs{"MANUAL=true", "NEXT_PUBLIC_VERCEL_ENV=true"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// set the env vars
			setEnvs(tt.env)
			// test
			res, err := GetHashableEnvVars(tt.args.envKeys, tt.args.envPrefixes, "TURBO_CI_VENDOR_ENV_KEY")
			if err != nil {
				t.Errorf("error setup failure: %s", err)
			}
			got := res.All.ToHashable()
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("got %#v, want %#v", got, tt.want)
			}
			// clean up the env for the next run
			os.Clearenv()
		})
	}
}

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
			want:             EnvironmentVariableMap{},
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
