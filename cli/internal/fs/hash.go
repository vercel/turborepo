package fs

import (
	"crypto/sha1"
	"encoding/hex"
	"fmt"
	"io"
	"strconv"

	"github.com/vercel/turbo/cli/internal/env"
	"github.com/vercel/turbo/cli/internal/lockfile"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"github.com/vercel/turbo/cli/internal/xxhash"
)

// LockfilePackages is a hashable list of packages
type LockfilePackages []lockfile.Package

// FileHashes is a hashable map of files to the hash of their contents
type FileHashes map[turbopath.AnchoredUnixPath]string

// TaskHashable is a hashable representation of a task to be run
type TaskHashable struct {
	GlobalHash           string
	TaskDependencyHashes []string
	PackageDir           turbopath.AnchoredUnixPath
	HashOfFiles          string
	ExternalDepsHash     string
	Task                 string
	Outputs              TaskOutputs
	PassThruArgs         []string
	Env                  []string
	ResolvedEnvVars      env.EnvironmentVariablePairs
	PassThroughEnv       []string
	EnvMode              util.EnvMode
	DotEnv               turbopath.AnchoredUnixPathArray
}

// GlobalHashable is a hashable representation of global dependencies for tasks
type GlobalHashable struct {
	GlobalCacheKey       string
	GlobalFileHashMap    map[turbopath.AnchoredUnixPath]string
	RootExternalDepsHash string
	Env                  []string
	ResolvedEnvVars      env.EnvironmentVariablePairs
	PassThroughEnv       []string
	EnvMode              util.EnvMode
	FrameworkInference   bool

	// NOTE! This field is _explicitly_ ordered and should not be sorted.
	DotEnv turbopath.AnchoredUnixPathArray
}

// HashLockfilePackages hashes a list of packages
func HashLockfilePackages(packages LockfilePackages) (string, error) {
	return hashObject(packages)
}

// HashFileHashes produces a single hash for a set of file hashes
func HashFileHashes(hashes FileHashes) (string, error) {
	return hashObject(hashes)
}

// HashTask produces the hash for a particular task
func HashTask(task *TaskHashable) (string, error) {
	// return proto.HashTaskHashable(task)
	return "", nil
}

// HashGlobal produces the global hash value to be incorporated in every task hash
func HashGlobal(global GlobalHashable) (string, error) {
	// return proto.HashGlobalHashable(&global)
	return "", nil
}

// hashObject is the internal generic hash function. It should not be used directly,
// but instead via a helper above to ensure that we are properly enumerating all of the
// the kinds of data that we hash.
func hashObject(i interface{}) (string, error) {
	hash := xxhash.New()

	_, err := hash.Write([]byte(fmt.Sprintf("%v", i)))

	return hex.EncodeToString(hash.Sum(nil)), err
}

// GitLikeHashFile is a function that mimics how Git
// calculates the SHA1 for a file (or, in Git terms, a "blob") (without git)
func GitLikeHashFile(filePath turbopath.AbsoluteSystemPath) (string, error) {
	file, err := filePath.Open()
	if err != nil {
		return "", err
	}
	defer file.Close()

	stat, err := file.Stat()
	if err != nil {
		return "", err
	}
	hash := sha1.New()
	hash.Write([]byte("blob"))
	hash.Write([]byte(" "))
	hash.Write([]byte(strconv.FormatInt(stat.Size(), 10)))
	hash.Write([]byte{0})

	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}

	return hex.EncodeToString(hash.Sum(nil)), nil
}
