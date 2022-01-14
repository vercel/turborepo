package util

import (
	"fmt"
	"io/ioutil"
	"path/filepath"
	"regexp"
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

func IsBerry(cwd string, version string) (bool, error) {
	v, err := semver.NewVersion(version)
	if err != nil {
		return false, fmt.Errorf("could not parse yarn version: %w", err)
	}
	c, err := semver.NewConstraint(">=2.0.0")
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

func GetPackageManagerAndVersion(packageManager string) (string, string) {
	re := regexp.MustCompile(`(npm|pnpm|yarn)@(\d+)\.\d+\.\d+(-.+)?`)
	match := re.FindString(packageManager)

	return strings.Split(match, "@")[0], strings.Split(match, "@")[1]
}
