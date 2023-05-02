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
	gitSetup(t, testDir)
	gitCheckoutBranch(t, testDir, targetbranch)
	branch := GetCurrentBranch(testDir)
	assert.Equal(t, branch, targetbranch)
	gitRm(t, testDir)
}

func TestGetCurrentBranchNonMain(t *testing.T) {
	targetbranch := "mybranch"
	testDir := getTestDir(t, "myrepo")
	gitSetup(t, testDir)
	gitCheckoutBranch(t, testDir, targetbranch)
	branch := GetCurrentBranch(testDir)
	assert.Equal(t, branch, targetbranch)
	gitRm(t, testDir)
}

func TestGetCurrentSHA(t *testing.T) {
	testDir := getTestDir(t, "myrepo")
	gitSetup(t, testDir)

	// initial sha is blank because there are no commits
	initSha := GetCurrentSha(testDir)
	assert.True(t, initSha == "", "initial sha is empty")

	// create new commit
	gitCommit(t, testDir)
	sha1 := GetCurrentSha(testDir)
	assert.True(t, sha1 != "sha on commit 1 is not empty")
	gitCommit(t, testDir)

	// second sha
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
	if _, err := cmd.Output(); err != nil {
		t.Fatalf("Failed to cleanup git dir: %v", err)
	}
}

func gitSetup(t *testing.T, dir turbopath.AbsoluteSystemPath) {
	cmd := exec.Command("git", []string{"init"}...)
	cmd.Dir = dir.ToString()
	if _, err := cmd.Output(); err != nil {
		t.Fatalf("Failed to checkout new branch in fixture repo: %v", err)
	}
}

func gitCommit(t *testing.T, dir turbopath.AbsoluteSystemPath) {
	cmd := exec.Command("git", []string{"commit", "--allow-empty", "-am", "new commit"}...)
	cmd.Dir = dir.ToString()
	if _, err := cmd.Output(); err != nil {
		t.Fatalf("Failed to checkout new branch in fixture repo: %v", err)
	}
}

func gitCheckoutBranch(t *testing.T, dir turbopath.AbsoluteSystemPath, branchname string) {
	// using capital -B instead of -b, so we avoid "branch already exists errors"
	cmd := exec.Command("git", []string{"checkout", "-B", branchname}...)
	cmd.Dir = dir.ToString()
	if _, err := cmd.Output(); err != nil {
		t.Fatalf("Failed to checkout new branch in fixture repo: %v", err)
	}
}
