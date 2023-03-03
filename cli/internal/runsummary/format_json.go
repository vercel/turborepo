package runsummary

import (
	"encoding/json"

	"github.com/pkg/errors"
)

// FormatJSON returns a json string representing a RunSummary
func (summary *RunSummary) FormatJSON(singlePackage bool) (string, error) {
	if singlePackage {
		return summary.formatJSONSinglePackage()
	}

	bytes, err := json.MarshalIndent(summary, "", "  ")
	if err != nil {
		return "", errors.Wrap(err, "failed to render JSON")
	}
	return string(bytes), nil
}

func (summary *RunSummary) formatJSONSinglePackage() (string, error) {
	singlePackageTasks := make([]singlePackageTaskSummary, len(summary.Tasks))

	for i, ht := range summary.Tasks {
		singlePackageTasks[i] = ht.toSinglePackageTask()
	}

	dryRun := &singlePackageRunSummary{singlePackageTasks}

	bytes, err := json.MarshalIndent(dryRun, "", "  ")
	if err != nil {
		return "", errors.Wrap(err, "failed to render JSON")
	}
	return string(bytes), nil
}
