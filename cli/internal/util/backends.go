package util

import (
	"fmt"
	"io/ioutil"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"github.com/vercel/turborepo/cli/internal/fs"

	"gopkg.in/yaml.v3"
)

type YarnRC struct {
	NodeLinker string `yaml:"nodeLinker"`
}

func IsYarn(backendName string) bool {
	return backendName == "nodejs-yarn" || backendName == "nodejs-berry"
}

func IsBerry(cwd string, version string, pkgManager bool) (bool, error) {
	if pkgManager {
		version, err := strconv.Atoi(strings.SplitN(version, ".", 2)[0])
		if err != nil {
			return false, err
		}

		if version >= 2 {
			return true, nil
		}

		return false, nil
	}

	return fs.PathExists(filepath.Join(cwd, ".yarnrc.yml")), nil
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

func GetPackageManagerAndVersion(packageManager string) (string, string, error) {
	pattern := `(npm|pnpm|yarn)@(\d+)\.\d+\.\d+(-.+)?`
	re := regexp.MustCompile(pattern)
	match := re.FindString(packageManager)
	if len(match) == 0 {
		return "", "", fmt.Errorf("could not parse packageManager field in package.json, expected: %s, received: %s", pattern, packageManager)
	}

	return strings.Split(match, "@")[0], strings.Split(match, "@")[1], nil
}
