package run

import (
	"reflect"
	"testing"
)

func Test_getHashableTurboEnvVarsFromOs(t *testing.T) {
	env := []string{
		"SOME_ENV_VAR=excluded",
		"SOME_OTHER_ENV_VAR=excluded",
		"FIRST_THASH_ENV_VAR=first",
		"TURBO_TOKEN=never",
		"SOME_OTHER_THASH_ENV_VAR=second",
		"TURBO_TEAM=never",
	}
	gotNames, gotPairs := getHashableTurboEnvVarsFromOs(env)
	wantNames := []string{"FIRST_THASH_ENV_VAR", "SOME_OTHER_THASH_ENV_VAR"}
	wantPairs := []string{"FIRST_THASH_ENV_VAR=first", "SOME_OTHER_THASH_ENV_VAR=second"}
	if !reflect.DeepEqual(wantNames, gotNames) {
		t.Errorf("getHashableTurboEnvVarsFromOs() env names got = %v, want %v", gotNames, wantNames)
	}
	if !reflect.DeepEqual(wantPairs, gotPairs) {
		t.Errorf("getHashableTurboEnvVarsFromOs() env pairs got = %v, want %v", gotPairs, wantPairs)
	}
}
