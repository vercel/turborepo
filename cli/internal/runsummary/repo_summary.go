package runsummary

import (
	"github.com/vercel/turbo/cli/internal/ci"
	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/scm"
)

type gitState struct {
	Sha    string `json:"sha"`
	Branch string `json:"branch"`
}

// getGitState returns the sha and branch when in a git repo
// Otherwise it should return empty strings right now.
// We my add handling of other scms and non-git tracking in the future.
func getGitState() *gitState {
	allEnvVars := env.GetEnvMap()

	gitstate := &gitState{}

	// If we're in CI, try to get the values we need from environment variables
	if ci.IsCi() {
		vendor := ci.Info()
		gitstate.Sha = allEnvVars[vendor.ShaEnvVar]
		gitstate.Branch = allEnvVars[vendor.BranchEnvVar]
	}

	// Otherwise fallback to using `git`
	if gitstate.Branch == "" {
		gitstate.Branch = scm.GetCurrentBranch()
	}
	if gitstate.Sha == "" {
		gitstate.Sha = scm.GetCurrentSha()
	}

	return gitstate
}
