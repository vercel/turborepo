package packagemanager

import (
	"reflect"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

func TestInferRoot(t *testing.T) {
	tests := []struct {
		name        string
		directory   turbopath.AbsoluteSystemPath
		rootPath    turbopath.AbsoluteSystemPath
		packageMode PackageType
	}{
		// TODO: Add test cases.
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, got1 := InferRoot(tt.directory)
			if !reflect.DeepEqual(got, tt.rootPath) {
				t.Errorf("getRootAndPackageMode() got = %v, want %v", got, tt.rootPath)
			}
			if got1 != tt.packageMode {
				t.Errorf("getRootAndPackageMode() got1 = %v, want %v", got1, tt.packageMode)
			}
		})
	}
}
