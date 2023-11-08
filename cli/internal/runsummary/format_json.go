package runsummary

import (
	"bytes"
	"encoding/json"
	"sort"

	"github.com/pkg/errors"
	"github.com/segmentio/ksuid"
	"github.com/vercel/turbo/cli/internal/util"
)

// FormatJSON returns a json string representing a RunSummary
func (rsm *Meta) FormatJSON() ([]byte, error) {
	rsm.normalize() // normalize data

	var err error
	var buffer bytes.Buffer

	encoder := json.NewEncoder(&buffer)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")

	if rsm.singlePackage {
		err = encoder.Encode(nonMonorepoRunSummary(*rsm.RunSummary))
	} else {
		err = encoder.Encode(rsm.RunSummary)
	}

	if err != nil {
		return nil, errors.Wrap(err, "failed to render JSON")
	}

	return buffer.Bytes(), nil
}

func (rsm *Meta) normalize() {
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

	sort.Sort(byTaskID(rsm.RunSummary.Tasks))
}

type byTaskID []*TaskSummary

func (a byTaskID) Len() int           { return len(a) }
func (a byTaskID) Swap(i, j int)      { a[i], a[j] = a[j], a[i] }
func (a byTaskID) Less(i, j int) bool { return a[i].TaskID < a[j].TaskID }

// nonMonorepoRunSummary is an exact copy of RunSummary, but the JSON tags are structured
// for rendering a single-package run of turbo. Notably, we want to always omit packages
// since there is no concept of packages in a single-workspace repo.
// This struct exists solely for the purpose of serializing to JSON and should not be
// used anywhere else.
type nonMonorepoRunSummary struct {
	ID                 ksuid.KSUID        `json:"id"`
	Version            string             `json:"version"`
	TurboVersion       string             `json:"turboVersion"`
	Monorepo           bool               `json:"monorepo"`
	GlobalHashSummary  *GlobalHashSummary `json:"globalCacheInputs"`
	Packages           []string           `json:"-"`
	EnvMode            util.EnvMode       `json:"envMode"`
	FrameworkInference bool               `json:"frameworkInference"`
	ExecutionSummary   *executionSummary  `json:"execution,omitempty"`
	Tasks              []*TaskSummary     `json:"tasks"`
	User               string             `json:"user"`
	SCM                *scmState          `json:"scm"`
}
