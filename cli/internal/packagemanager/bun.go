package packagemanager

import (
	"fmt"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

var nodejsBun = PackageManager{
	Name:       "nodejs-bun",
	Slug:       "bun",
	Command:    "bun",
	Specfile:   "package.json",
	Lockfile:   "bun.lockb",
	PackageDir: "node_modules",
	ArgSeparator: func(userArgs []string) []string {
		// Bun swallows a single "--" token. If the user is passing "--", we need
		// to prepend our own so that the user's doesn't get swallowed. If they are not
		// passing their own, we don't need the "--" token and can avoid the warning.
		for _, arg := range userArgs {
			if arg == "--" {
				return []string{"--"}
			}
		}
		return nil
	},

	getWorkspaceGlobs: func(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		pkg, err := fs.ReadPackageJSON(rootpath.UntypedJoin("package.json"))
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		if len(pkg.Workspaces) == 0 {
			return nil, fmt.Errorf("package.json: no workspaces found. Turborepo requires Bun workspaces to be defined in the root package.json")
		}
		return pkg.Workspaces, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		// Matches upstream values:
		// Key code: https://github.com/oven-sh/bun/blob/f267c1d097923a2d2992f9f60a6dd365fe706512/src/install/lockfile.zig#L3057
		return []string{
			"**/node_modules",
			"**/.git",
		}, nil
	},

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return false, nil
	},

	UnmarshalLockfile: func(_rootPackageJSON *fs.PackageJSON, contents []byte) (lockfile.Lockfile, error) {
		return lockfile.DecodeBunLockfile(contents)
	},
}
