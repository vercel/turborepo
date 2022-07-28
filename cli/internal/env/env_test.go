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

func TestGetHashableEnvPairs(t *testing.T) {
	type args struct {
		envKeys []string
	}
	tests := []struct {
		env  []string
		name string
		args args
		want []string
	}{
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, no env values",
			args: args{
				envKeys: []string{"myval"},
			},
			want: []string{"myval="},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, one env value",
			args: args{
				envKeys: []string{"lowercase"},
			},
			want: []string{"lowercase=stillcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "lowercase=notcool"},
			name: "no framework env vars, duplicate env value",
			args: args{
				envKeys: []string{"lowercase"},
			},
			want: []string{"lowercase=notcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers"},
			name: "no framework env vars, multiple env values",
			args: args{
				envKeys: []string{"lowercase", "MY_TEST_VAR"},
			},
			want: []string{"MY_TEST_VAR=cool", "lowercase=stillcool"},
		},
		{
			env:  []string{"lowercase=stillcool", "MY_TEST_VAR=cool", "12345=numbers", "NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "one framework env var, multiple env values",
			args: args{
				envKeys: []string{"lowercase", "MY_TEST_VAR"},
			},
			want: []string{"MY_TEST_VAR=cool", "NEXT_PUBLIC_MY_COOL_VAR=cool", "lowercase=stillcool"},
		},
		{
			env:  []string{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
			name: "duplicate framework env var and env values",
			args: args{
				envKeys: []string{"NEXT_PUBLIC_MY_COOL_VAR"},
			},
			want: []string{"NEXT_PUBLIC_MY_COOL_VAR=cool"},
		},
		{
			env:  []string{"a=1", "b=2", "c=3", "PUBLIC_myvar=4"},
			name: "sorts correctly",
			args: args{
				envKeys: []string{"a", "b", "c"},
			},
			want: []string{"PUBLIC_myvar=4", "a=1", "b=2", "c=3"},
		},
		{
			env:  []string{"a=1=2", "NEXT_PUBLIC_VALUE_TEST=do=not=do=this"},
			name: "parses env values correctly",
			args: args{
				envKeys: []string{"a"},
			},
			want: []string{"NEXT_PUBLIC_VALUE_TEST=do=not=do=this", "a=1=2"},
		},
		{
			env:  []string{"a=1", "NEXT_PUBLIC_=weird"},
			name: "parses prefix with no ending",
			args: args{
				envKeys: []string{"a"},
			},
			want: []string{"NEXT_PUBLIC_=weird", "a=1"},
		},
		{
			env:  []string{"NEXT_PUBLIC_EMOJI=😋"},
			name: "parses unicode env value",
			args: args{
				envKeys: []string{},
			},
			want: []string{"NEXT_PUBLIC_EMOJI=😋"},
		},
		{
			env:  []string{"zero=0", "null=null", "nil=nil"},
			name: "parses corner case env values",
			args: args{
				envKeys: []string{"zero", "null", "nil"},
			},
			want: []string{"nil=nil", "null=null", "zero=0"},
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
				envKeys: []string{},
			},
			want: []string{"GATSBY_custom=GATSBY",
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
				envKeys: []string{"FINAL", "CUSTOM", "ANOTHER"},
			},
			want: []string{"ANOTHER=neat", "CUSTOM=cool", "FINAL=great", "GATSBY_custom=GATSBY",
				"NEXT_PUBLIC_custom=NEXT_PUBLIC",
				"NUXT_ENV_custom=NUXT_ENV",
				"PUBLIC_custom=PUBLIC",
				"REACT_APP_custom=REACT_APP",
				"REDWOOD_ENV_custom=REDWOOD_ENV",
				"SANITY_STUDIO_custom=SANITY_STUDIO",
				"VITE_custom=VITE",
				"VUE_APP_custom=VUE_APP"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// set the env vars
			setEnvs(tt.env)
			// test
			if got := GetHashableEnvPairs(tt.args.envKeys); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GetHashableEnvPairs() = %v, want %v", got, tt.want)
			}
			// clean up the env for the next run
			os.Clearenv()
		})
	}
}
