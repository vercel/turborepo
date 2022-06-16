package config

import (
	"fmt"

	"github.com/Masterminds/semver"
	"github.com/vercel/turborepo/cli/internal/fs"
)

// CheckTurboVersionCompatibility makes sure that the Turbo version is compatible with the configuration
func CheckTurboVersionCompatibility(turboVersion string, c *Config) error {
	v, err := semver.NewVersion(turboVersion)
	if err != nil {
		panic(err)
	}
	err = checkPackageTurboEngineConstraint(v, c.RootPackageJSON)
	if err != nil {
		return err
	}
	return nil
}

func checkPackageTurboEngineConstraint(turboVersion *semver.Version, packageJSON *fs.PackageJSON) error {
	// The lack of an engine constraint means there's nothing to validate and isn't an error.
	if packageJSON == nil || packageJSON.Engines["turbo"] == "" {
		return nil
	}
	c, err := semver.NewConstraint(packageJSON.Engines["turbo"])
	if err != nil {
		return fmt.Errorf("package.json: the 'engines.turbo' constraint is not valid")
	}
	if !c.Check(turboVersion) {
		return fmt.Errorf("package.json: version '%v' of Turbo does not meet the '%v' engine constraint", turboVersion, packageJSON.Engines["turbo"])
	}
	return nil
}
