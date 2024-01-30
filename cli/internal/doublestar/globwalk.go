// Package doublestar is adapted from https://github.com/bmatcuk/doublestar
// Copyright Bob Matcuk. All Rights Reserved.
// SPDX-License-Identifier: MIT
package doublestar

import (
	"io/fs"
	"path"
)

// GlobWalkFunc is a callback function for GlobWalk(). If the function returns an error, GlobWalk
// will end immediately and return the same error.
type GlobWalkFunc func(path string, d fs.DirEntry) error

// GlobWalk calls the callback function `fn` for every file matching pattern.
// The syntax of pattern is the same as in Match() and the behavior is the same
// as Glob(), with regard to limitations (such as patterns containing `/./`,
// `/../`, or starting with `/`). The pattern may describe hierarchical names
// such as usr/*/bin/ed.
//
// GlobWalk may have a small performance benefit over Glob if you do not need a
// slice of matches because it can avoid allocating memory for the matches.
// Additionally, GlobWalk gives you access to the `fs.DirEntry` objects for
// each match, and lets you quit early by returning a non-nil error from your
// callback function.
//
// GlobWalk ignores file system errors such as I/O errors reading directories.
// GlobWalk may return ErrBadPattern, reporting that the pattern is malformed.
// Additionally, if the callback function `fn` returns an error, GlobWalk will
// exit immediately and return that error.
//
// Like Glob(), this function assumes that your pattern uses `/` as the path
// separator even if that's not correct for your OS (like Windows). If you
// aren't sure if that's the case, you can use filepath.ToSlash() on your
// pattern before calling GlobWalk().
func GlobWalk(fsys fs.FS, pattern string, fn GlobWalkFunc) error {
	if !ValidatePattern(pattern) {
		return ErrBadPattern
	}
	return doGlobWalk(fsys, pattern, true, fn)
}

// Actually execute GlobWalk
func doGlobWalk(fsys fs.FS, pattern string, firstSegment bool, fn GlobWalkFunc) error {
	patternStart := indexMeta(pattern)
	if patternStart == -1 {
		// pattern doesn't contain any meta characters - does a file matching the
		// pattern exist?
		info, err := fs.Stat(fsys, pattern)
		if err == nil {
			err = fn(pattern, newDirEntryFromFileInfo(info))
			return err
		}
		// ignore IO errors
		return nil
	}

	dir := "."
	splitIdx := lastIndexSlashOrAlt(pattern)
	if splitIdx != -1 {
		if pattern[splitIdx] == '}' {
			openingIdx := indexMatchedOpeningAlt(pattern[:splitIdx])
			if openingIdx == -1 {
				// if there's no matching opening index, technically Match() will treat
				// an unmatched `}` as nothing special, so... we will, too!
				splitIdx = lastIndexSlash(pattern[:splitIdx])
			} else {
				// otherwise, we have to handle the alts:
				return globAltsWalk(fsys, pattern, openingIdx, splitIdx, firstSegment, fn)
			}
		}

		dir = pattern[:splitIdx]
		pattern = pattern[splitIdx+1:]
	}

	// if `splitIdx` is less than `patternStart`, we know `dir` has no meta
	// characters. They would be equal if they are both -1, which means `dir`
	// will be ".", and we know that doesn't have meta characters either.
	if splitIdx <= patternStart {
		return globDirWalk(fsys, dir, pattern, firstSegment, fn)
	}

	return doGlobWalk(fsys, dir, false, func(p string, d fs.DirEntry) error {
		if err := globDirWalk(fsys, p, pattern, firstSegment, fn); err != nil {
			return err
		}
		return nil
	})
}

// handle alts in the glob pattern - `openingIdx` and `closingIdx` are the
// indexes of `{` and `}`, respectively
func globAltsWalk(fsys fs.FS, pattern string, openingIdx, closingIdx int, firstSegment bool, fn GlobWalkFunc) error {
	var matches []dirEntryWithFullPath
	startIdx := 0
	afterIdx := closingIdx + 1
	splitIdx := lastIndexSlashOrAlt(pattern[:openingIdx])
	if splitIdx == -1 || pattern[splitIdx] == '}' {
		// no common prefix
		var err error
		matches, err = doGlobAltsWalk(fsys, "", pattern, startIdx, openingIdx, closingIdx, afterIdx, firstSegment, matches)
		if err != nil {
			return err
		}
	} else {
		// our alts have a common prefix that we can process first
		startIdx = splitIdx + 1
		err := doGlobWalk(fsys, pattern[:splitIdx], false, func(p string, d fs.DirEntry) (e error) {
			matches, e = doGlobAltsWalk(fsys, p, pattern, startIdx, openingIdx, closingIdx, afterIdx, firstSegment, matches)
			return e
		})
		if err != nil {
			return err
		}
	}

	for _, m := range matches {
		if err := fn(m.Path, m.Entry); err != nil {
			return err
		}
	}

	return nil
}

// runs actual matching for alts
func doGlobAltsWalk(fsys fs.FS, d, pattern string, startIdx, openingIdx, closingIdx, afterIdx int, firstSegment bool, m []dirEntryWithFullPath) ([]dirEntryWithFullPath, error) {
	matches := m
	matchesLen := len(m)
	patIdx := openingIdx + 1
	for patIdx < closingIdx {
		nextIdx := indexNextAlt(pattern[patIdx:closingIdx], true)
		if nextIdx == -1 {
			nextIdx = closingIdx
		} else {
			nextIdx += patIdx
		}

		alt := buildAlt(d, pattern, startIdx, openingIdx, patIdx, nextIdx, afterIdx)
		err := doGlobWalk(fsys, alt, firstSegment, func(p string, d fs.DirEntry) error {
			// insertion sort, ignoring dups
			insertIdx := matchesLen
			for insertIdx > 0 && matches[insertIdx-1].Path > p {
				insertIdx--
			}
			if insertIdx > 0 && matches[insertIdx-1].Path == p {
				// dup
				return nil
			}

			// append to grow the slice, then insert
			entry := dirEntryWithFullPath{d, p}
			matches = append(matches, entry)
			for i := matchesLen; i > insertIdx; i-- {
				matches[i] = matches[i-1]
			}
			matches[insertIdx] = entry
			matchesLen++

			return nil
		})
		if err != nil {
			return nil, err
		}

		patIdx = nextIdx + 1
	}

	return matches, nil
}

func globDirWalk(fsys fs.FS, dir, pattern string, canMatchFiles bool, fn GlobWalkFunc) error {
	if pattern == "" {
		// pattern can be an empty string if the original pattern ended in a slash,
		// in which case, we should just return dir, but only if it actually exists
		// and it's a directory (or a symlink to a directory)
		info, err := fs.Stat(fsys, dir)
		if err != nil || !info.IsDir() {
			return nil
		}
		return fn(dir, newDirEntryFromFileInfo(info))
	}

	if pattern == "**" {
		// `**` can match *this* dir
		info, err := fs.Stat(fsys, dir)
		if err != nil || !info.IsDir() {
			return nil
		}
		if err = fn(dir, newDirEntryFromFileInfo(info)); err != nil {
			return err
		}
		return globDoubleStarWalk(fsys, dir, canMatchFiles, fn)
	}

	dirs, err := fs.ReadDir(fsys, dir)
	if err != nil {
		// ignore IO errors
		return nil
	}

	var matched bool
	for _, info := range dirs {
		name := info.Name()
		if canMatchFiles || isDir(fsys, dir, name, info) {
			matched, err = matchWithSeparator(pattern, name, '/', false)
			if err != nil {
				return err
			}
			if matched {
				if err = fn(path.Join(dir, name), info); err != nil {
					return err
				}
			}
		}
	}

	return nil
}

func globDoubleStarWalk(fsys fs.FS, dir string, canMatchFiles bool, fn GlobWalkFunc) error {
	dirs, err := fs.ReadDir(fsys, dir)
	if err != nil {
		// ignore IO errors
		return nil
	}

	// `**` can match *this* dir, so add it
	for _, info := range dirs {
		name := info.Name()
		if isDir(fsys, dir, name, info) {
			p := path.Join(dir, name)
			if e := fn(p, info); e != nil {
				return e
			}
			if e := globDoubleStarWalk(fsys, p, canMatchFiles, fn); e != nil {
				return e
			}
		} else if canMatchFiles {
			if e := fn(path.Join(dir, name), info); e != nil {
				return e
			}
		}
	}

	return nil
}

type dirEntryFromFileInfo struct {
	fi fs.FileInfo
}

func (d *dirEntryFromFileInfo) Name() string {
	return d.fi.Name()
}

func (d *dirEntryFromFileInfo) IsDir() bool {
	return d.fi.IsDir()
}

func (d *dirEntryFromFileInfo) Type() fs.FileMode {
	return d.fi.Mode().Type()
}

func (d *dirEntryFromFileInfo) Info() (fs.FileInfo, error) {
	return d.fi, nil
}

func newDirEntryFromFileInfo(fi fs.FileInfo) fs.DirEntry {
	return &dirEntryFromFileInfo{fi}
}

type dirEntryWithFullPath struct {
	Entry fs.DirEntry
	Path  string
}
