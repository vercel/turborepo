package runsummary

import (
	"encoding/json"

	"github.com/pkg/errors"
)

// FormatJSON returns a json string representing a RunSummary
func (rsm *Meta) FormatJSON() ([]byte, error) {
	rsm.normalize() // normalize data

	bytes, err := json.MarshalIndent(rsm.RunSummary, "", "  ")
	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}
	return bytes, nil
}

func (rsm *Meta) normalize() {
	for _, t := range rsm.RunSummary.Tasks {
		t.EnvVars.Global = rsm.RunSummary.GlobalHashSummary.EnvVars
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
