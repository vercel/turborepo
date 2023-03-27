package runsummary

import (
	"encoding/json"

	"github.com/pkg/errors"
)

// FormatJSON returns a json string representing a RunSummary
func (rsm *Meta) FormatJSON() ([]byte, error) {
	rsm.RunSummary.normalize() // normalize data

	if rsm.singlePackage {
		return rsm.formatJSONSinglePackage()
	}

	bytes, err := json.MarshalIndent(rsm.RunSummary, "", "  ")
	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}
	return bytes, nil
}

func (rsm *Meta) formatJSONSinglePackage() ([]byte, error) {
	singlePackageTasks := make([]singlePackageTaskSummary, len(rsm.RunSummary.Tasks))

	for i, task := range rsm.RunSummary.Tasks {
		singlePackageTasks[i] = task.toSinglePackageTask()
	}

	spSummary := &singlePackageRunSummary{Tasks: singlePackageTasks}

	bytes, err := json.MarshalIndent(spSummary, "", "  ")
	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}

	return bytes, nil
}
