package runsummary

import (
	"encoding/json"

	"github.com/pkg/errors"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/util"
)

// FormatJSON returns a json string representing a RunSummary
func (rsm *Meta) FormatJSON() ([]byte, error) {
	rsm.normalize() // normalize data

	var bytes []byte
	var err error

	if rsm.singlePackage {
		bytes, err = json.MarshalIndent(nonMonorepoRunSummary(*rsm.RunSummary), "", "  ")
	} else {
		bytes, err = json.MarshalIndent(rsm.RunSummary, "", "  ")
	}

	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}
	return bytes, nil
}

func (rsm *Meta) normalize() {
	for _, t := range rsm.RunSummary.Tasks {
		t.EnvVars.Global = rsm.RunSummary.GlobalHashSummary.envVars
		t.EnvVars.GlobalPassthrough = rsm.RunSummary.GlobalHashSummary.passthroughEnvVars
	}

	// Remove execution summary for dry runs
	if rsm.runType == runTypeDryJSON {
		rsm.RunSummary.ExecutionSummary = nil
	}

	// For single packages, we don't need the Packages
	// and each task summary needs some cleaning.
	if rsm.singlePackage {
		rsm.RunSummary.Packages = []string{}

		for _, task := range rsm.RunSummary.Tasks {
			task.cleanForSinglePackage()
		}
	}
}

// nonMonorepoRunSummary is an exact copy of RunSummary, but the JSON tags are structured
// for rendering a single-package run of turbo. Notably, we want to always omit packages
// since there is no concept of packages in a single-workspace repo.
// This struct exists solely for the purpose of serializing to JSON and should not be
// used anywhere else.
type nonMonorepoRunSummary struct {
	ID                ksuid.KSUID        `json:"id"`
	Version           string             `json:"version"`
	TurboVersion      string             `json:"turboVersion"`
	GlobalHashSummary *GlobalHashSummary `json:"globalCacheInputs"`
	Packages          []string           `json:"-"`
	EnvMode           util.EnvMode       `json:"envMode"`
	ExecutionSummary  *executionSummary  `json:"execution,omitempty"`
	Tasks             []*TaskSummary     `json:"tasks"`
}
