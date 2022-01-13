package util

import (
	"fmt"
	"io/ioutil"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/Masterminds/semver"
	"gopkg.in/yaml.v3"
)

type YarnRC struct {
	NodeLinker string `yaml:"nodeLinker"`
}

func IsYarn(backendName string) bool {
	return backendName == "nodejs-yarn" || backendName == "nodejs-berry"
}

func IsBerry(cwd string) (bool, error) {
	cmd := exec.Command("yarn", "--version")
	cmd.Dir = cwd
	out, err := cmd.Output()
	if err != nil {
		return false, fmt.Errorf("could not detect yarn version: %w", err)
	}

	v, err := semver.NewVersion(strings.TrimSpace(string(out)))
	if err != nil {
		return false, fmt.Errorf("could not parse yarn version: %w", err)
	}
	c, err := semver.NewConstraint(">= 2.0.0")
	if err != nil {
		return false, fmt.Errorf("could not create constraint: %w", err)
	}

	return c.Check(v), nil
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
