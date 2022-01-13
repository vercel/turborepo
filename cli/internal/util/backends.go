package util

import (
	"fmt"
	"os/exec"
	"strings"

	"github.com/Masterminds/semver"
)

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
