package runsummary

import (
	"encoding/json"

	"github.com/pkg/errors"
)

// FormatJSON returns a json string representing a RunSummary
func (summary *RunSummary) FormatJSON(singlePackage bool) ([]byte, error) {
	if singlePackage {
		return summary.formatJSONSinglePackage()
	}

	bytes, err := json.MarshalIndent(summary, "", "  ")
	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}
	return bytes, nil
}

func (summary *RunSummary) formatJSONSinglePackage() ([]byte, error) {
	singlePackageTasks := make([]singlePackageTaskSummary, len(summary.Tasks))

	for i, task := range summary.Tasks {
		singlePackageTasks[i] = task.toSinglePackageTask()
	}

	spSummary := &singlePackageRunSummary{Tasks: singlePackageTasks}

	bytes, err := json.MarshalIndent(spSummary, "", "  ")
	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}

	return bytes, nil
}
