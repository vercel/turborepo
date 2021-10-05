package nodejs

import (
	"fmt"
	"io/ioutil"

	"turbo/internal/api"
	"turbo/internal/fs"

	"gopkg.in/yaml.v2"
)

// nodejsPatterns is the FilenamePatterns value for NodejsBackend.
var nodejsPatterns = []string{"*.js", ".mjs", "*.ts", "*.jsx", "*.tsx"}

var NodejsYarnBackend = api.LanguageBackend{
	Name:             "nodejs-yarn",
	Specfile:         "package.json",
	Lockfile:         "yarn.lock",
	FilenamePatterns: nodejsPatterns,
	GetWorkspaceGlobs: func() ([]string, error) {
		pkg, err := fs.ReadPackageJSON("package.json")
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		return pkg.Workspaces, nil
	},
	GetPackageDir: func() string {
		return "node_modules"
	},
	GetRunCommand: func() []string {
		return []string{"yarn", "run"}
	},
}

// PnpmWorkspaces is a representation of workspace package globs found
// in pnpm-workspace.yaml
type PnpmWorkspaces struct {
	Packages []string `yaml:"packages,omitempty"`
}

var NodejsPnpmBackend = api.LanguageBackend{
	Name:             "nodejs-pnpm",
	Specfile:         "package.json",
	Lockfile:         "pnpm-lock.yaml",
	FilenamePatterns: nodejsPatterns,
	GetWorkspaceGlobs: func() ([]string, error) {
		bytes, err := ioutil.ReadFile("pnpm-workspace.yaml")
		if err != nil {
			return nil, fmt.Errorf("pnpm-workspace.yaml: %w", err)
		}
		var pnpmWorkspaces PnpmWorkspaces
		if err := yaml.Unmarshal(bytes, &pnpmWorkspaces); err != nil {
			return nil, fmt.Errorf("pnpm-workspace.yaml: %w", err)
		}
		return pnpmWorkspaces.Packages, nil
	},
	GetPackageDir: func() string {
		return "node_modules"
	},
	GetRunCommand: func() []string {
		return []string{"pnpm", "run"}
	},
}

var NodejsNpmBackend = api.LanguageBackend{
	Name:             "nodejs-npm",
	Specfile:         "package.json",
	Lockfile:         "package-lock.json",
	FilenamePatterns: nodejsPatterns,
	GetWorkspaceGlobs: func() ([]string, error) {
		pkg, err := fs.ReadPackageJSON("package.json")
		if err != nil {
			return nil, fmt.Errorf("package.json: %w", err)
		}
		return pkg.Workspaces, nil
	},
	GetPackageDir: func() string {
		return "node_modules"
	},
	GetRunCommand: func() []string {
		return []string{"npm", "run"}
	},
}
