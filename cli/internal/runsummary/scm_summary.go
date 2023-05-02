package runsummary

import (
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/scm"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

type scmState struct {
	Type   string `json:"type"`
	Sha    string `json:"sha"`
	Branch string `json:"branch"`
}

// getSCMState returns the sha and branch when in a git repo
// Otherwise it should return empty strings right now.
// We my add handling of other scms and non-git tracking in the future.
func getSCMState(dir turbopath.AbsoluteSystemPath) *scmState {
	allEnvVars := env.GetEnvMap()

	state := &scmState{Type: "git"}

	// If we're in CI, try to get the values we need from environment variables
	if ci.IsCi() {
		vendor := ci.Info()
		state.Sha = allEnvVars[vendor.ShaEnvVar]
		state.Branch = allEnvVars[vendor.BranchEnvVar]
	}

	// Otherwise fallback to using `git`
	if state.Branch == "" {
		state.Branch = scm.GetCurrentBranch(dir)
	}

	if state.Sha == "" {
		state.Sha = scm.GetCurrentSha(dir)
	}

	return state
}
