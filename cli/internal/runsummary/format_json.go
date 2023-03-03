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

	for i, task := range summary.Tasks {
		singlePackageTasks[i] = task.toSinglePackageTask()
	}

	spSummary := &singlePackageRunSummary{Tasks: singlePackageTasks}

	bytes, err := json.MarshalIndent(spSummary, "", "  ")
	if err != nil {
		return "", errors.Wrap(err, "failed to render JSON")
	}

	return string(bytes), nil
}
