package util

import (
	"fmt"
	"io/ioutil"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/Masterminds/semver"
	"github.com/pkg/errors"
	"gopkg.in/yaml.v3"
)

type YarnRC struct {
	NodeLinker string `yaml:"nodeLinker"`
}

func IsYarn(backendName string) bool {
	return backendName == "nodejs-yarn" || backendName == "nodejs-berry"
}

func IsNMLinker(cwd string) (bool, error) {
	yarnRC := &YarnRC{}

	bytes, err := ioutil.ReadFile(filepath.Join(cwd, ".yarnrc.yml"))
	if err != nil {
		return false, fmt.Errorf(".yarnrc.yml: %w", err)
	}

	if yaml.Unmarshal(bytes, yarnRC) != nil {
		return false, fmt.Errorf(".yarnrc.yml: %w", err)
	}

	return yarnRC.NodeLinker == "node-modules", nil
}

// mustCompileSemverConstraint compiles the given text into a constraint
// and panics on error. Intended for uses where an error indicates a programming
// error and we should crash ASAP.
func mustCompileSemverConstraint(text string) *semver.Constraints {
	c, err := semver.NewConstraint(text)
	if err != nil {
		panic(err)
	}
	return c
}

var _pnpmPre7Constraint = mustCompileSemverConstraint(">=7.0.0")

// Is7PlusPnpm returns true if the given backend is both nodejs-pnpm *AND*
// is version >=7.0.0
func Is7PlusPnpm(backedName string) (bool, error) {
	if backedName == "nodejs-pnpm" {
		out, err := exec.Command("pnpm", "--version").CombinedOutput()
		if err != nil {
			return false, err
		}
		versionRaw := strings.TrimSpace(string(out))
		version, err := semver.NewVersion(versionRaw)
		if err != nil {
			return false, errors.Wrapf(err, "parsing semver for %v", versionRaw)
		}
		return _pnpmPre7Constraint.Check(version), nil
	}
	return false, nil
}
