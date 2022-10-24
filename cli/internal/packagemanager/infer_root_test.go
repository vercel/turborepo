package packagemanager

import (
	"reflect"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func TestInferRoot(t *testing.T) {
	type file struct {
		path    turbopath.AnchoredSystemPath
		content []byte
	}

	tests := []struct {
		name               string
		fs                 []file
		executionDirectory turbopath.AnchoredSystemPath
		rootPath           turbopath.AnchoredSystemPath
		packageMode        PackageType
	}{
		// Scenario 0
		{
			name: "turbo.json at current dir, no package.json",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "turbo.json at parent dir, no package.json",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			// This is "no inference"
			rootPath:    turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			packageMode: Multi,
		},
		// Scenario 1A
		{
			name: "turbo.json at current dir, has package.json, has workspaces key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "turbo.json at parent dir, has package.json, has workspaces key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "turbo.json at parent dir, has package.json, has pnpm workspaces",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("pnpm-workspace.yaml").ToSystemPath(),
					content: []byte("packages:\n  - docs"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		// Scenario 1A aware of the weird thing we do for packages.
		{
			name: "turbo.json at current dir, has package.json, has packages key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"packages\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Single,
		},
		{
			name: "turbo.json at parent dir, has package.json, has packages key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"packages\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Single,
		},
		// Scenario 1A aware of the the weird thing we do for packages when both methods of specification exist.
		{
			name: "turbo.json at current dir, has package.json, has workspace and packages key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"clobbered\" ], \"packages\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "turbo.json at parent dir, has package.json, has workspace and packages key",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"clobbered\" ], \"packages\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		// Scenario 1B
		{
			name: "turbo.json at current dir, has package.json, no workspaces",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{}"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Single,
		},
		{
			name: "turbo.json at parent dir, has package.json, no workspaces",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{}"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Single,
		},
		{
			name: "turbo.json at parent dir, has package.json, no workspaces, includes pnpm",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{path: turbopath.AnchoredUnixPath("turbo.json").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("pnpm-workspace.yaml").ToSystemPath(),
					content: []byte(""),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Single,
		},
		// Scenario 2A
		{
			name:               "no turbo.json, no package.json at current",
			fs:                 []file{},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "no turbo.json, no package.json at parent",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			packageMode:        Multi,
		},
		// Scenario 2B
		{
			name: "no turbo.json, has package.json with workspaces at current",
			fs: []file{
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "no turbo.json, has package.json with workspaces at parent",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"exists\" ] }"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			packageMode:        Multi,
		},
		{
			name: "no turbo.json, has package.json with pnpm workspaces at parent",
			fs: []file{
				{path: turbopath.AnchoredUnixPath("execution/path/subdir/.file").ToSystemPath()},
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"exists\" ] }"),
				},
				{
					path:    turbopath.AnchoredUnixPath("pnpm-workspace.yaml").ToSystemPath(),
					content: []byte("packages:\n  - docs"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("execution/path/subdir").ToSystemPath(),
			packageMode:        Multi,
		},
		// Scenario 3A
		{
			name: "no turbo.json, lots of package.json files but no workspaces",
			fs: []file{
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/three/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			packageMode:        Single,
		},
		// Scenario 3BI
		{
			name: "no turbo.json, lots of package.json files, and a workspace at the root that matches execution directory",
			fs: []file{
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"one/two/three\" ] }"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/three/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			packageMode:        Multi,
		},
		// Scenario 3BII
		{
			name: "no turbo.json, lots of package.json files, and a workspace at the root that matches execution directory",
			fs: []file{
				{
					path:    turbopath.AnchoredUnixPath("package.json").ToSystemPath(),
					content: []byte("{ \"workspaces\": [ \"does-not-exist\" ] }"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
				{
					path:    turbopath.AnchoredUnixPath("one/two/three/package.json").ToSystemPath(),
					content: []byte("{}"),
				},
			},
			executionDirectory: turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			rootPath:           turbopath.AnchoredUnixPath("one/two/three").ToSystemPath(),
			packageMode:        Single,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			fsRoot := turbopath.AbsoluteSystemPath(t.TempDir())
			for _, file := range tt.fs {
				path := file.path.RestoreAnchor(fsRoot)
				assert.NilError(t, path.Dir().MkdirAll(0777))
				assert.NilError(t, path.WriteFile(file.content, 0777))
			}

			turboRoot, packageMode := InferRoot(tt.executionDirectory.RestoreAnchor(fsRoot))
			if !reflect.DeepEqual(turboRoot, tt.rootPath.RestoreAnchor(fsRoot)) {
				t.Errorf("InferRoot() turboRoot = %v, want %v", turboRoot, tt.rootPath.RestoreAnchor(fsRoot))
			}
			if packageMode != tt.packageMode {
				t.Errorf("InferRoot() packageMode = %v, want %v", packageMode, tt.packageMode)
			}
		})
	}
}
