package cacheitem

import (
	"reflect"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
)

func Test_cachedDirTree_getStartingPoint(t *testing.T) {
	testDir := turbopath.AbsoluteSystemPath("")
	tests := []struct {
		name string

		// STATE
		cachedDirTree cachedDirTree

		// INPUT
		path turbopath.AnchoredSystemPath

		// OUTPUT
		calculatedAnchor turbopath.AbsoluteSystemPath
		pathSegments     []turbopath.RelativeSystemPath
	}{
		{
			name: "hello world",
			cachedDirTree: cachedDirTree{
				anchorAtDepth: []turbopath.AbsoluteSystemPath{testDir},
				prefix:        []turbopath.RelativeSystemPath{},
			},
			path:             turbopath.AnchoredUnixPath("hello/world").ToSystemPath(),
			calculatedAnchor: testDir,
			pathSegments:     []turbopath.RelativeSystemPath{"hello", "world"},
		},
		{
			name: "has a cache",
			cachedDirTree: cachedDirTree{
				anchorAtDepth: []turbopath.AbsoluteSystemPath{
					testDir,
					testDir.UntypedJoin("hello"),
				},
				prefix: []turbopath.RelativeSystemPath{"hello"},
			},
			path:             turbopath.AnchoredUnixPath("hello/world").ToSystemPath(),
			calculatedAnchor: testDir.UntypedJoin("hello"),
			pathSegments:     []turbopath.RelativeSystemPath{"world"},
		},
		{
			name: "ask for yourself",
			cachedDirTree: cachedDirTree{
				anchorAtDepth: []turbopath.AbsoluteSystemPath{
					testDir,
					testDir.UntypedJoin("hello"),
					testDir.UntypedJoin("hello", "world"),
				},
				prefix: []turbopath.RelativeSystemPath{"hello", "world"},
			},
			path:             turbopath.AnchoredUnixPath("hello/world").ToSystemPath(),
			calculatedAnchor: testDir.UntypedJoin("hello", "world"),
			pathSegments:     []turbopath.RelativeSystemPath{},
		},
		{
			name: "three layer cake",
			cachedDirTree: cachedDirTree{
				anchorAtDepth: []turbopath.AbsoluteSystemPath{
					testDir,
					testDir.UntypedJoin("hello"),
					testDir.UntypedJoin("hello", "world"),
				},
				prefix: []turbopath.RelativeSystemPath{"hello", "world"},
			},
			path:             turbopath.AnchoredUnixPath("hello/world/again").ToSystemPath(),
			calculatedAnchor: testDir.UntypedJoin("hello", "world"),
			pathSegments:     []turbopath.RelativeSystemPath{"again"},
		},
		{
			name: "outside of cache hierarchy",
			cachedDirTree: cachedDirTree{
				anchorAtDepth: []turbopath.AbsoluteSystemPath{
					testDir,
					testDir.UntypedJoin("hello"),
					testDir.UntypedJoin("hello", "world"),
				},
				prefix: []turbopath.RelativeSystemPath{"hello", "world"},
			},
			path:             turbopath.AnchoredUnixPath("somewhere/else").ToSystemPath(),
			calculatedAnchor: testDir,
			pathSegments:     []turbopath.RelativeSystemPath{"somewhere", "else"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cr := tt.cachedDirTree
			calculatedAnchor, pathSegments := cr.getStartingPoint(tt.path)
			if !reflect.DeepEqual(calculatedAnchor, tt.calculatedAnchor) {
				t.Errorf("cachedDirTree.getStartingPoint() calculatedAnchor = %v, want %v", calculatedAnchor, tt.calculatedAnchor)
			}
			if !reflect.DeepEqual(pathSegments, tt.pathSegments) {
				t.Errorf("cachedDirTree.getStartingPoint() pathSegments = %v, want %v", pathSegments, tt.pathSegments)
			}
		})
	}
}
