package fs

import (
	"fmt"
	iofs "io/fs"
	"io/ioutil"
	"os"
	"path/filepath"
	"reflect"

	"github.com/spf13/pflag"
)

// The goal is to be able to teach the Go type system about six
// different types of paths:
// - AbsoluteSystemPath
// - RelativeSystemPath
// - RepoRelativeSystemPath
// - AbsoluteUnixPath
// - RelativeUnixPath
// - RepoRelativeUnixPath
//
// There is a second portion which allows clustering particular
// dimensions of these four types:
// - AbsolutePathInterface
// - RelativePathInterface
// - RepoRelativePathInterface
// - UnixPathInterface
// - SystemPathInterface
// - FilePathInterface
//
// Between these two things it is assumed that we will be able to
// reasonably describe file paths being used within the system and
// have the type system enforce correctness instead of relying upon
// runtime code to accomplish the task.
//
// Much of this is dreadfully repetitive because of intentional
// limitations in the Go type system.

// AbsoluteSystemPath is a root-relative path using system separators.
type AbsoluteSystemPath string

// RelativeSystemPath is a relative path using system separators.
type RelativeSystemPath string

// RepoRelativeSystemPath is a relative path from the repository using system separators.
type RepoRelativeSystemPath string

// AbsoluteUnixPath is a root-relative path using Unix `/` separators.
type AbsoluteUnixPath string

// RelativeUnixPath is a relative path using Unix `/` separators.
type RelativeUnixPath string

// RepoRelativeUnixPath is a relative path from the repository using Unix `/` separators.
type RepoRelativeUnixPath string

// AbsoluteUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type AbsoluteUnixPathInterface interface {
	filePathStamp()
	absolutePathStamp()
	unixPathStamp()

	ToString() string
}

// RelativeUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RelativeUnixPathInterface interface {
	filePathStamp()
	relativePathStamp()
	unixPathStamp()

	ToString() string
}

// RepoRelativeUnixPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RepoRelativeUnixPathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()
	unixPathStamp()

	ToString() string
}

// AbsoluteSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type AbsoluteSystemPathInterface interface {
	filePathStamp()
	absolutePathStamp()
	systemPathStamp()

	ToString() string
}

// RelativeSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RelativeSystemPathInterface interface {
	filePathStamp()
	relativePathStamp()
	systemPathStamp()

	ToString() string
}

// RepoRelativeSystemPathInterface specifies the members that mark an interface
// for structurally typing so the Go compiler can understand it.
type RepoRelativeSystemPathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()
	systemPathStamp()

	ToString() string
}

// AbsolutePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type AbsolutePathInterface interface {
	filePathStamp()
	absolutePathStamp()

	ToString() string
}

// RelativePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type RelativePathInterface interface {
	filePathStamp()
	relativePathStamp()

	ToString() string
}

// RepoRelativePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type RepoRelativePathInterface interface {
	filePathStamp()
	relativePathStamp()
	repoRelativeStamp()

	ToString() string
}

// UnixPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type UnixPathInterface interface {
	filePathStamp()
	unixPathStamp()

	Rel(UnixPathInterface) (RelativeUnixPath, error)
	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// SystemPathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type SystemPathInterface interface {
	filePathStamp()
	systemPathStamp()

	Rel(SystemPathInterface) (RelativeSystemPath, error)
	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// FilePathInterface specifies additional dimensions that a particular
// object can have that are independent from each other.
type FilePathInterface interface {
	filePathStamp()

	ToSystemPath() SystemPathInterface
	ToUnixPath() UnixPathInterface
	ToString() string
}

// For interface reasons, we need a way to distinguish between
// Absolute/Repo/Relative/System/Unix/File paths so we stamp them.
func (AbsoluteUnixPath) absolutePathStamp()   {}
func (AbsoluteSystemPath) absolutePathStamp() {}

func (RelativeUnixPath) relativePathStamp()       {}
func (RelativeSystemPath) relativePathStamp()     {}
func (RepoRelativeUnixPath) relativePathStamp()   {}
func (RepoRelativeSystemPath) relativePathStamp() {}

func (AbsoluteUnixPath) unixPathStamp()     {}
func (RelativeUnixPath) unixPathStamp()     {}
func (RepoRelativeUnixPath) unixPathStamp() {}

func (AbsoluteSystemPath) systemPathStamp()     {}
func (RelativeSystemPath) systemPathStamp()     {}
func (RepoRelativeSystemPath) systemPathStamp() {}

func (AbsoluteUnixPath) filePathStamp()       {}
func (RelativeUnixPath) filePathStamp()       {}
func (RepoRelativeUnixPath) filePathStamp()   {}
func (AbsoluteSystemPath) filePathStamp()     {}
func (RelativeSystemPath) filePathStamp()     {}
func (RepoRelativeSystemPath) filePathStamp() {}

// StringToUnixPath parses a path string from an unknown source
// and converts it into a Unix path. The only valid separator for
// a result from this call is `/`
func StringToUnixPath(path string) UnixPathInterface {
	if filepath.IsAbs(path) {
		return AbsoluteUnixPath(filepath.ToSlash(path))
	}
	return RelativeUnixPath(filepath.ToSlash(path))
}

// StringToSystemPath parses a path string from an unknown source
// and converts it into a system path. This means it could have
// any valid separators (`\` or `/`).
func StringToSystemPath(path string) SystemPathInterface {
	if filepath.IsAbs(path) {
		return AbsoluteSystemPath(filepath.FromSlash(path))
	}
	return RelativeSystemPath(filepath.FromSlash(path))
}

// ToSystemPath converts a AbsoluteUnixPath to a SystemPath.
func (p AbsoluteUnixPath) ToSystemPath() SystemPathInterface {
	return AbsoluteSystemPath(filepath.FromSlash(p.ToString()))
}

// ToSystemPath converts a RelativeUnixPath to a SystemPath.
func (p RelativeUnixPath) ToSystemPath() SystemPathInterface {
	return RelativeSystemPath(filepath.FromSlash(p.ToString()))
}

// ToSystemPath converts a RelativeUnixPath to a SystemPath.
func (p RepoRelativeUnixPath) ToSystemPath() SystemPathInterface {
	return RepoRelativeSystemPath(filepath.FromSlash(p.ToString()))
}

// ToSystemPath called on a AbsoluteSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p AbsoluteSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToSystemPath called on a RelativeSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p RelativeSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToSystemPath called on a RelativeSystemPath returns itself.
// It exists to enable simpler code at call sites.
func (p RepoRelativeSystemPath) ToSystemPath() SystemPathInterface {
	return p
}

// ToUnixPath called on a AbsoluteUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p AbsoluteUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// ToUnixPath called on a RelativeUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p RelativeUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// ToUnixPath called on a RepoRelativeUnixPath returns itself.
// It exists to enable simpler code at call sites.
func (p RepoRelativeUnixPath) ToUnixPath() UnixPathInterface {
	return p
}

// ToUnixPath converts a AbsoluteSystemPath to a UnixPath.
func (p AbsoluteSystemPath) ToUnixPath() UnixPathInterface {
	return AbsoluteUnixPath(filepath.ToSlash(p.ToString()))
}

// ToUnixPath converts a RelativeSystemPath to a UnixPath.
func (p RelativeSystemPath) ToUnixPath() UnixPathInterface {
	return RelativeUnixPath(filepath.ToSlash(p.ToString()))
}

// ToUnixPath converts a RepoRelativeSystemPath to a UnixPath.
func (p RepoRelativeSystemPath) ToUnixPath() UnixPathInterface {
	return RepoRelativeUnixPath(filepath.ToSlash(p.ToString()))
}

// Rel calculates the relative path between a AbsoluteUnixPath and any other UnixPath.
func (p AbsoluteUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// Rel calculates the relative path between a RelativeUnixPath and any other UnixPath.
func (p RelativeUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// Rel calculates the relative path between a RepoRelativeUnixPath and any other UnixPath.
func (p RepoRelativeUnixPath) Rel(basePath UnixPathInterface) (RelativeUnixPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeUnixPath(processed), err
}

// Rel calculates the relative path between a AbsoluteSystemPath and any other SystemPath.
func (p AbsoluteSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// Rel calculates the relative path between a RelativeSystemPath and any other SystemPath.
func (p RelativeSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// Rel calculates the relative path between a RelativeSystemPath and any other SystemPath.
func (p RepoRelativeSystemPath) Rel(basePath SystemPathInterface) (RelativeSystemPath, error) {
	processed, err := filepath.Rel(basePath.ToString(), p.ToString())
	return RelativeSystemPath(processed), err
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteUnixPath) ToString() string {
	return string(p)
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeUnixPath) ToString() string {
	return string(p)
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RepoRelativeUnixPath) ToString() string {
	return string(p)
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p AbsoluteSystemPath) ToString() string {
	return string(p)
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RelativeSystemPath) ToString() string {
	return string(p)
}

// ToString returns a string represenation of this Path.
// Used for interfacing with APIs that require a string.
func (p RepoRelativeSystemPath) ToString() string {
	return string(p)
}

// ToRelativeUnixPath converts from RelativeSystemPath to RelativeUnixPath.
func (p RelativeSystemPath) ToRelativeUnixPath() RelativeUnixPath {
	return p.ToUnixPath().(RelativeUnixPath)
}

// ToRelativeSystemPath converts from RelativeUnixPath to RelativeSystemPath.
func (p RelativeUnixPath) ToRelativeSystemPath() RelativeSystemPath {
	return p.ToSystemPath().(RelativeSystemPath)
}

// ToRepoRelativeUnixPath converts from RelativeSystemPath to RepoRelativeUnixPath.
func (p RepoRelativeSystemPath) ToRepoRelativeUnixPath() RepoRelativeUnixPath {
	return p.ToUnixPath().(RepoRelativeUnixPath)
}

// ToRepoRelativeSystemPath converts from RelativeUnixPath to RelativeSystemPath.
func (p RepoRelativeUnixPath) ToRepoRelativeSystemPath() RepoRelativeSystemPath {
	return p.ToSystemPath().(RepoRelativeSystemPath)
}

// ToRelativeSystemPath converts from RepoRelativeSystemPath to RelativeSystemPath.
func (p RepoRelativeSystemPath) ToRelativeSystemPath() RelativeSystemPath {
	return RelativeSystemPath(p)
}

// ToRelativeUnixPath converts from RepoRelativeUnixPath to RelativeSystemPath.
func (p RepoRelativeUnixPath) ToRelativeUnixPath() RelativeUnixPath {
	return RelativeUnixPath(p)
}

// ToAbsoluteUnixPath converts from AbsoluteSystemPath to AbsoluteUnixPath.
func (p AbsoluteSystemPath) ToAbsoluteUnixPath() AbsoluteUnixPath {
	return p.ToUnixPath().(AbsoluteUnixPath)
}

// ToAbsoluteSystemPath converts from AbsoluteUnixPath to AbsoluteSystemPath.
func (p AbsoluteUnixPath) ToAbsoluteSystemPath() AbsoluteSystemPath {
	return p.ToSystemPath().(AbsoluteSystemPath)
}

// UnsafeToRelativeUnixPath ingests an arbitrary string and treats it as
// a RepoRelativeUnixPath.
func UnsafeToRelativeUnixPath(s string) RelativeUnixPath {
	return RelativeUnixPath(s)
}

// AbsolutePath represents a platform-dependent absolute path on the filesystem,
// and is used to enfore correct path manipulation
type AbsolutePath string

func CheckedToAbsolutePath(s string) (AbsolutePath, error) {
	if filepath.IsAbs(s) {
		return AbsolutePath(s), nil
	}
	return "", fmt.Errorf("%v is not an absolute path", s)
}

// ResolveUnknownPath returns unknown if it is an absolute path, otherwise, it
// assumes unknown is a path relative to the given root.
func ResolveUnknownPath(root AbsolutePath, unknown string) AbsolutePath {
	if filepath.IsAbs(unknown) {
		return AbsolutePath(unknown)
	}
	return root.Join(unknown)
}

func UnsafeToAbsolutePath(s string) AbsolutePath {
	return AbsolutePath(s)
}

// AbsolutePathFromUpstream is used to mark return values from APIs that we
// expect to give us absolute paths. No checking is performed.
// Prefer to use this over a cast to maintain the search-ability of interfaces
// into and out of the AbsolutePath type.
func AbsolutePathFromUpstream(s string) AbsolutePath {
	return AbsolutePath(s)
}

func GetCwd() (AbsolutePath, error) {
	cwdRaw, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("invalid working directory: %w", err)
	}
	// We evaluate symlinks here because the package managers
	// we support do the same.
	cwdRaw, err = filepath.EvalSymlinks(cwdRaw)
	if err != nil {
		return "", fmt.Errorf("evaluating symlinks in cwd: %w", err)
	}
	cwd, err := CheckedToAbsolutePath(cwdRaw)
	if err != nil {
		return "", fmt.Errorf("cwd is not an absolute path %v: %v", cwdRaw, err)
	}
	return cwd, nil
}

func (ap AbsolutePath) ToStringDuringMigration() string {
	return ap.asString()
}

func (ap AbsolutePath) Join(args ...string) AbsolutePath {
	return AbsolutePath(filepath.Join(ap.asString(), filepath.Join(args...)))
}
func (ap AbsolutePath) asString() string {
	return string(ap)
}
func (ap AbsolutePath) Dir() AbsolutePath {
	return AbsolutePath(filepath.Dir(ap.asString()))
}

// MkdirAll is the AbsolutePath wrapper for os.MkdirAll
func (ap AbsolutePath) MkdirAll() error {
	return os.MkdirAll(ap.asString(), DirPermissions|0644)
}
func (ap AbsolutePath) Open() (*os.File, error) {
	return os.Open(ap.asString())
}

// OpenFile implements os.OpenFile for an absolute path
func (ap AbsolutePath) OpenFile(flags int, mode os.FileMode) (*os.File, error) {
	return os.OpenFile(ap.asString(), flags, mode)
}

func (ap AbsolutePath) FileExists() bool {
	return FileExists(ap.asString())
}

// Lstat implements os.Lstat for absolute path
func (ap AbsolutePath) Lstat() (os.FileInfo, error) {
	return os.Lstat(ap.asString())
}

// DirExists returns true if this path points to a directory
func (ap AbsolutePath) DirExists() bool {
	info, err := os.Lstat(ap.asString())
	return err == nil && info.IsDir()
}

// ContainsPath returns true if this absolute path is a parent of the
// argument.
func (ap AbsolutePath) ContainsPath(other AbsolutePath) (bool, error) {
	return DirContainsPath(ap.asString(), other.asString())
}

// ReadFile reads the contents of the specified file
func (ap AbsolutePath) ReadFile() ([]byte, error) {
	return ioutil.ReadFile(ap.asString())
}

// WriteFile writes the contents of the specified file
func (ap AbsolutePath) WriteFile(contents []byte, mode os.FileMode) error {
	return ioutil.WriteFile(ap.asString(), contents, mode)
}

// EnsureDir ensures that the directory containing this file exists
func (ap AbsolutePath) EnsureDir() error {
	return EnsureDir(ap.asString())
}

// Create is the AbsolutePath wrapper for os.Create
func (ap AbsolutePath) Create() (*os.File, error) {
	return os.Create(ap.asString())
}

// Ext implements filepath.Ext for an absolute path
func (ap AbsolutePath) Ext() string {
	return filepath.Ext(ap.asString())
}

// ToString returns the string representation of this absolute path. Used for
// interfacing with APIs that require a string
func (ap AbsolutePath) ToString() string {
	return ap.asString()
}

// RelativePathString returns the relative path from this AbsolutePath to another absolute path in string form as a string
func (ap AbsolutePath) RelativePathString(path string) (string, error) {
	return filepath.Rel(ap.asString(), path)
}

// Symlink implements os.Symlink(target, ap) for absolute path
func (ap AbsolutePath) Symlink(target string) error {
	return os.Symlink(target, ap.asString())
}

// Remove removes the file or (empty) directory at the given path
func (ap AbsolutePath) Remove() error {
	return os.Remove(ap.asString())
}

// Rename implements os.Rename for absolute paths
func (ap AbsolutePath) Rename(dest AbsolutePath) error {
	return os.Rename(ap.asString(), dest.asString())
}

// GetVolumeRoot returns the root directory given an absolute path.
func GetVolumeRoot(absolutePath string) string {
	return filepath.VolumeName(absolutePath) + string(os.PathSeparator)
}

// CreateDirFSAtRoot creates an `os.dirFS` instance at the root of the
// volume containing the specified path.
func CreateDirFSAtRoot(absolutePath string) iofs.FS {
	return os.DirFS(GetVolumeRoot(absolutePath))
}

// GetDirFSRootPath returns the root path of a os.dirFS.
func GetDirFSRootPath(fsys iofs.FS) string {
	// We can't typecheck fsys to enforce using an `os.dirFS` because the
	// type isn't exported from `os`. So instead, reflection. 🤷‍♂️

	fsysType := reflect.TypeOf(fsys).Name()
	if fsysType != "dirFS" {
		// This is not a user error, fail fast
		panic("GetDirFSRootPath must receive an os.dirFS")
	}

	// The underlying type is a string; this is the original path passed in.
	return reflect.ValueOf(fsys).String()
}

// IofsRelativePath calculates a `os.dirFS`-friendly path from an absolute system path.
func IofsRelativePath(fsysRoot string, absolutePath string) (string, error) {
	return filepath.Rel(fsysRoot, absolutePath)
}

type pathValue struct {
	base     AbsolutePath
	current  *AbsolutePath
	defValue string
}

func (pv *pathValue) String() string {
	if *pv.current == "" {
		return ResolveUnknownPath(pv.base, pv.defValue).ToString()
	}
	return pv.current.ToString()
}

func (pv *pathValue) Set(value string) error {
	*pv.current = ResolveUnknownPath(pv.base, value)
	return nil
}

func (pv *pathValue) Type() string {
	return "path"
}

var _ pflag.Value = &pathValue{}

// AbsolutePathVar adds a flag interpreted as an absolute path to the given FlagSet.
// It currently requires a root because relative paths are interpreted relative to the
// given root.
func AbsolutePathVar(flags *pflag.FlagSet, target *AbsolutePath, name string, root AbsolutePath, usage string, defValue string) {
	value := &pathValue{
		base:     root,
		current:  target,
		defValue: defValue,
	}
	flags.Var(value, name, usage)
}
