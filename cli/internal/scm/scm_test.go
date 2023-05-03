package scm

import (
	"os"
	"os/exec"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
)

func TestGetCurrentBranchMain(t *testing.T) {
	targetbranch := "main"
	testDir := getTestDir(t, "myrepo")

	// Setup git
	gitCommand(t, testDir, []string{"config", "--local", "user.email", "turbo@vercel.com"})
	gitCommand(t, testDir, []string{"config", "--local", "user.name", "Turbobot"})
	gitCommand(t, testDir, []string{"init"})

	gitCommand(t, testDir, []string{"checkout", "-B", targetbranch})
	branch := GetCurrentBranch(testDir)
	assert.Equal(t, branch, targetbranch)

	// cleanup
	gitRm(t, testDir)
}

func TestGetCurrentBranchNonMain(t *testing.T) {
	targetbranch := "mybranch"
	testDir := getTestDir(t, "myrepo")
	// Setup git
	gitCommand(t, testDir, []string{"config", "--local", "user.email", "turbo@vercel.com"})
	gitCommand(t, testDir, []string{"config", "--local", "user.name", "Turbobot"})
	gitCommand(t, testDir, []string{"init"})
	gitCommand(t, testDir, []string{"checkout", "-B", targetbranch})

	branch := GetCurrentBranch(testDir)
	assert.Equal(t, branch, targetbranch)

	// cleanup
	gitRm(t, testDir)
}

func TestGetCurrentSHA(t *testing.T) {
	testDir := getTestDir(t, "myrepo")
	// Setup git
	gitCommand(t, testDir, []string{"config", "--local", "user.email", "turbo@vercel.com"})
	gitCommand(t, testDir, []string{"config", "--local", "user.name", "Turbobot"})
	gitCommand(t, testDir, []string{"init"})

	// initial sha is blank because there are no commits
	initSha := GetCurrentSha(testDir)
	assert.True(t, initSha == "", "initial sha is empty")

	// first commit
	gitCommand(t, testDir, []string{"commit", "--allow-empty", "-am", "new commit"})
	sha1 := GetCurrentSha(testDir)
	assert.True(t, sha1 != "sha on commit 1 is not empty")

	// second commit
	gitCommand(t, testDir, []string{"commit", "--allow-empty", "-am", "new commit"})
	sha2 := GetCurrentSha(testDir)
	assert.True(t, sha2 != "", "sha on commit 2 is not empty")
	assert.True(t, sha2 != sha1, "sha on commit 2 changes from commit 1")

	// cleanup
	gitRm(t, testDir)
}

func getTestDir(t *testing.T, testName string) turbopath.AbsoluteSystemPath {
	defaultCwd, err := os.Getwd()
	if err != nil {
		t.Errorf("failed to get cwd: %v", err)
	}
	cwd, err := fs.CheckedToAbsoluteSystemPath(defaultCwd)
	if err != nil {
		t.Fatalf("cwd is not an absolute directory %v: %v", defaultCwd, err)
	}

	return cwd.UntypedJoin("testdata", testName)
}

func gitRm(t *testing.T, dir turbopath.AbsoluteSystemPath) {
	cmd := exec.Command("rm", []string{"-rf", ".git"}...)
	cmd.Dir = dir.ToString()
	if out, err := cmd.Output(); err != nil {
		t.Fatalf("Failed to cleanup git dir: %s\n%v", out, err)
	}
}

func gitCommand(t *testing.T, cwd turbopath.AbsoluteSystemPath, args []string) {
	cmd := exec.Command("git", args...)
	cmd.Dir = cwd.ToString()
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("Failed to checkout new branch: %s\n%v", out, err)
	}
}
