//go:build go || !rust
// +build go !rust

package hashing

import (
	"reflect"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_memoizedGetTraversePath(t *testing.T) {
	fixturePath := getFixture(1)

	gotOne, _ := memoizedGetTraversePath(fixturePath)
	gotTwo, _ := memoizedGetTraversePath(fixturePath)

	assert.Check(t, gotOne == gotTwo, "The strings are identical.")
}

func Test_getTraversePath(t *testing.T) {
	fixturePath := getFixture(1)

	tests := []struct {
		name     string
		rootPath turbopath.AbsoluteSystemPath
		want     turbopath.RelativeUnixPath
		wantErr  bool
	}{
		{
			name:     "From fixture location",
			rootPath: fixturePath,
			want:     turbopath.RelativeUnixPath("../../../"),
			wantErr:  false,
		},
		{
			name:     "Traverse out of git repo",
			rootPath: fixturePath.UntypedJoin("..", "..", "..", ".."),
			want:     "",
			wantErr:  true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := getTraversePath(tt.rootPath)
			if (err != nil) != tt.wantErr {
				t.Errorf("getTraversePath() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("getTraversePath() = %v, want %v", got, tt.want)
			}
		})
	}
}
