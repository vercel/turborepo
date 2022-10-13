package lockfile

import (
	"bytes"
	"fmt"
	"io"

	"github.com/andybalholm/crlf"
	"github.com/iseki0/go-yarnlock"
	"github.com/pkg/errors"
	"github.com/vercel/turborepo/cli/internal/turbopath"
)

var _crlfLiteral = []byte("\r\n")

// YarnLockfile representation of yarn lockfile
type YarnLockfile struct {
	inner   yarnlock.LockFile
	hasCRLF bool
}

var _ Lockfile = (*YarnLockfile)(nil)

// ResolvePackage Given a package and version returns the key, resolved version, and if it was found
func (l *YarnLockfile) ResolvePackage(_workspacePath turbopath.AnchoredUnixPath, name string, version string) (Package, error) {
	for _, key := range yarnPossibleKeys(name, version) {
		if entry, ok := (l.inner)[key]; ok {
			return Package{
				Found:   true,
				Key:     key,
				Version: entry.Version,
			}, nil
		}
	}

	return Package{}, nil
}

// AllDependencies Given a lockfile key return all (dev/optional/peer) dependencies of that package
func (l *YarnLockfile) AllDependencies(key string) (map[string]string, bool) {
	deps := map[string]string{}
	entry, ok := (l.inner)[key]
	if !ok {
		return deps, false
	}

	for name, version := range entry.Dependencies {
		deps[name] = version
	}
	for name, version := range entry.OptionalDependencies {
		deps[name] = version
	}

	return deps, true
}

// Subgraph Given a list of lockfile keys returns a Lockfile based off the original one that only contains the packages given
func (l *YarnLockfile) Subgraph(_ []turbopath.AnchoredSystemPath, packages []string) (Lockfile, error) {
	lockfile := make(map[string]yarnlock.LockFileEntry, len(packages))
	for _, key := range packages {
		entry, ok := (l.inner)[key]
		if ok {
			lockfile[key] = entry
		}
	}

	return &YarnLockfile{lockfile, l.hasCRLF}, nil
}

// Encode encode the lockfile representation and write it to the given writer
func (l *YarnLockfile) Encode(w io.Writer) error {
	writer := w
	if l.hasCRLF {
		writer = crlf.NewWriter(w)
	}
	if err := l.inner.Encode(writer); err != nil {
		return errors.Wrap(err, "Unable to encode yarn.lock")
	}
	return nil
}

// Patches return a list of patches used in the lockfile
func (l *YarnLockfile) Patches() []turbopath.AnchoredUnixPath {
	return nil
}

// DecodeYarnLockfile Takes the contents of a yarn lockfile and returns a struct representation
func DecodeYarnLockfile(contents []byte) (*YarnLockfile, error) {
	lockfile, err := yarnlock.ParseLockFileData(contents)
	hasCRLF := bytes.HasSuffix(contents, _crlfLiteral)
	newline := []byte("\n")

	// there's no trailing newline for this file, need to inspect more to see newline style
	if !hasCRLF && !bytes.HasSuffix(contents, newline) {
		firstNewline := bytes.IndexByte(contents, newline[0])
		if firstNewline != -1 && firstNewline != 0 {
			byteBeforeNewline := contents[firstNewline-1]
			hasCRLF = byteBeforeNewline == '\r'
		}
	}

	if err != nil {
		return nil, errors.Wrap(err, "Unable to decode yarn.lock")
	}

	return &YarnLockfile{lockfile, hasCRLF}, nil
}

func yarnPossibleKeys(name string, version string) []string {
	return []string{
		fmt.Sprintf("%v@%v", name, version),
		fmt.Sprintf("%v@npm:%v", name, version),
		fmt.Sprintf("%v@file:%v", name, version),
		fmt.Sprintf("%v@workspace:%v", name, version),
		fmt.Sprintf("%v@yarn:%v", name, version),
	}
}
