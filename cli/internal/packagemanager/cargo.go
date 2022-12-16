package packagemanager

import (
	"fmt"
	"github.com/pelletier/go-toml/v2"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

type CargoToml struct {
	Workspace CargoWorkspace `toml:"workspace"`
}

type CargoWorkspace struct {
	Members []string `toml:"members"`
	Exclude []string `toml:"exclude"`
}

var cargo = PackageManager{
	Name:       "cargo",
	Slug:       "cargo",
	Command:    "cargo",
	Specfile:   "Cargo.toml",
	Lockfile:   "Cargo.lock",
	PackageDir: "target/debug/deps",

	getWorkspaceGlobs: func(rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		cargoTomlText, err := rootpath.UntypedJoin("Cargo.toml").ReadFile()

		if err != nil {
			return nil, fmt.Errorf("Cargo.toml: %w", err)
		}

		var cargoToml CargoToml
		err = toml.Unmarshal(cargoTomlText, &cargoToml)
		if err != nil {
			return nil, fmt.Errorf("Cargo.toml: %w", err)
		}

		if len(cargoToml.Workspace.Members) == 0 {
			return nil, fmt.Errorf("Cargo.toml: no workspaces found. Turborepo requires Cargo workspaces to be defined in the root Cargo.toml")
		}

		return cargoToml.Workspace.Members, nil
	},

	getWorkspaceIgnores: func(pm PackageManager, rootpath turbopath.AbsoluteSystemPath) ([]string, error) {
		cargoTomlText, err := rootpath.UntypedJoin("Cargo.toml").ReadFile()

		if err != nil {
			return nil, fmt.Errorf("Cargo.toml: %w", err)
		}

		var cargoToml CargoToml
		err = toml.Unmarshal(cargoTomlText, &cargoToml)
		if err != nil {
			return nil, fmt.Errorf("Cargo.toml: %w", err)
		}

		return cargoToml.Workspace.Exclude, nil
	},

	canPrune: func(cwd turbopath.AbsoluteSystemPath) (bool, error) {
		return false, nil
	},

	Matches: func(manager string, version string) (bool, error) {
		return manager == "cargo", nil
	},

	detect: func(projectDirectory turbopath.AbsoluteSystemPath, packageManager *PackageManager) (bool, error) {
		hasCargoToml := projectDirectory.UntypedJoin(packageManager.Specfile).FileExists()

		return hasCargoToml, nil
	},

	readLockfile: func(contents []byte) (lockfile.Lockfile, error) {
		return nil, fmt.Errorf("Cargo.lock: reading lockfile not implemented")
	},

	prunePatches: func(pkgJSON *fs.PackageJSON, patches []turbopath.AnchoredUnixPath) error {
		return fmt.Errorf("Cargo.lock: pruning patches not implemented")
	},
}
