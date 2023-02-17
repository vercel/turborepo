package util

import (
	"fmt"
	"io/ioutil"
	"path/filepath"

	"github.com/vercel/turbo/cli/internal/yaml"
)

// YarnRC Represents contents of .yarnrc.yml
type YarnRC struct {
	NodeLinker string `yaml:"nodeLinker"`
}

// IsNMLinker Checks that Yarn is set to use the node-modules linker style
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
