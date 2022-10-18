package util

import (
	"fmt"
	"io/ioutil"
	"path/filepath"

	"github.com/vercel/turborepo/cli/internal/yaml"
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
