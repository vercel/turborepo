// Package doublestar is adapted from https://github.com/bmatcuk/doublestar
// Copyright Bob Matcuk. All Rights Reserved.
// SPDX-License-Identifier: MIT
package doublestar

import (
	"io/fs"
	"path"
)

// Glob returns the names of all files matching pattern or nil if there is no
// matching file. The syntax of pattern is the same as in Match(). The pattern
// may describe hierarchical names such as usr/*/bin/ed.
//
// Glob ignores file system errors such as I/O errors reading directories.
// The only possible returned error is ErrBadPattern, reporting that the
// pattern is malformed.
//
// Note: this is meant as a drop-in replacement for io/fs.Glob(). Like
// io/fs.Glob(), this function assumes that your pattern uses `/` as the path
// separator even if that's not correct for your OS (like Windows). If you
// aren't sure if that's the case, you can use filepath.ToSlash() on your
// pattern before calling Glob().
//
// Like `io/fs.Glob()`, patterns containing `/./`, `/../`, or starting with `/`
// will return no results and no errors. You can use SplitPattern to divide a
// pattern into a base path (to initialize an `FS` object) and pattern.
func Glob(fsys fs.FS, pattern string) ([]string, error) {
	if !ValidatePattern(pattern) {
		return nil, ErrBadPattern
	}
	if hasMidDoubleStar(pattern) {
		// If the pattern has a `**` anywhere but the very end, GlobWalk is more
		// performant because it can get away with less allocations. If the pattern
		// ends in a `**`, both methods are pretty much the same, but Glob has a
		// _very_ slight advantage because of lower function call overhead.
		var matches []string
		err := doGlobWalk(fsys, pattern, true, func(p string, d fs.DirEntry) error {
			matches = append(matches, p)
			return nil
		})
		return matches, err
	}
	return doGlob(fsys, pattern, nil, true)
}

// Does the actual globbin'
func doGlob(fsys fs.FS, pattern string, m []string, firstSegment bool) ([]string, error) {
	matches := m
	patternStart := indexMeta(pattern)
	if patternStart == -1 {
		// pattern doesn't contain any meta characters - does a file matching the
		// pattern exist?
		if exists(fsys, pattern) {
			matches = append(matches, pattern)
		}
		return matches, nil
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
				return globAlts(fsys, pattern, openingIdx, splitIdx, matches, firstSegment)
			}
		}

		dir = pattern[:splitIdx]
		pattern = pattern[splitIdx+1:]
	}

	// if `splitIdx` is less than `patternStart`, we know `dir` has no meta
	// characters. They would be equal if they are both -1, which means `dir`
	// will be ".", and we know that doesn't have meta characters either.
	if splitIdx <= patternStart {
		return globDir(fsys, dir, pattern, matches, firstSegment)
	}

	var dirs []string
	var err error
	dirs, err = doGlob(fsys, dir, matches, false)
	if err != nil {
		return nil, err
	}
	for _, d := range dirs {
		matches, err = globDir(fsys, d, pattern, matches, firstSegment)
		if err != nil {
			return nil, err
		}
	}

	return matches, nil
}

// handle alts in the glob pattern - `openingIdx` and `closingIdx` are the
// indexes of `{` and `}`, respectively
func globAlts(fsys fs.FS, pattern string, openingIdx, closingIdx int, m []string, firstSegment bool) ([]string, error) {
	matches := m

	var dirs []string
	startIdx := 0
	afterIdx := closingIdx + 1
	splitIdx := lastIndexSlashOrAlt(pattern[:openingIdx])
	if splitIdx == -1 || pattern[splitIdx] == '}' {
		// no common prefix
		dirs = []string{""}
	} else {
		// our alts have a common prefix that we can process first
		var err error
		dirs, err = doGlob(fsys, pattern[:splitIdx], matches, false)
		if err != nil {
			return nil, err
		}

		startIdx = splitIdx + 1
	}

	for _, d := range dirs {
		patIdx := openingIdx + 1
		altResultsStartIdx := len(matches)
		thisResultStartIdx := altResultsStartIdx
		for patIdx < closingIdx {
			nextIdx := indexNextAlt(pattern[patIdx:closingIdx], true)
			if nextIdx == -1 {
				nextIdx = closingIdx
			} else {
				nextIdx += patIdx
			}

			alt := buildAlt(d, pattern, startIdx, openingIdx, patIdx, nextIdx, afterIdx)
			var err error
			matches, err = doGlob(fsys, alt, matches, firstSegment)
			if err != nil {
				return nil, err
			}

			matchesLen := len(matches)
			if altResultsStartIdx != thisResultStartIdx && thisResultStartIdx != matchesLen {
				// Alts can result in matches that aren't sorted, or, worse, duplicates
				// (consider the trivial pattern `path/to/{a,*}`). Since doGlob returns
				// sorted results, we can do a sort of in-place merge and remove
				// duplicates. But, we only need to do this if this isn't the first alt
				// (ie, `altResultsStartIdx != thisResultsStartIdx`) and if the latest
				// alt actually added some matches (`thisResultStartIdx !=
				// len(matches)`)
				matches = sortAndRemoveDups(matches, altResultsStartIdx, thisResultStartIdx, matchesLen)

				// length of matches may have changed
				thisResultStartIdx = len(matches)
			} else {
				thisResultStartIdx = matchesLen
			}

			patIdx = nextIdx + 1
		}
	}

	return matches, nil
}

// find files/subdirectories in the given `dir` that match `pattern`
func globDir(fsys fs.FS, dir, pattern string, matches []string, canMatchFiles bool) ([]string, error) {
	m := matches

	if pattern == "" {
		// pattern can be an empty string if the original pattern ended in a slash,
		// in which case, we should just return dir, but only if it actually exists
		// and it's a directory (or a symlink to a directory)
		if isPathDir(fsys, dir) {
			m = append(m, dir)
		}
		return m, nil
	}

	if pattern == "**" {
		m = globDoubleStar(fsys, dir, m, canMatchFiles)
		return m, nil
	}

	dirs, err := fs.ReadDir(fsys, dir)
	if err != nil {
		// ignore IO errors
		return m, nil
	}

	var matched bool
	for _, info := range dirs {
		name := info.Name()
		if canMatchFiles || isDir(fsys, dir, name, info) {
			matched, err = matchWithSeparator(pattern, name, '/', false)
			if err != nil {
				return nil, err
			}
			if matched {
				m = append(m, path.Join(dir, name))
			}
		}
	}

	return m, nil
}

func globDoubleStar(fsys fs.FS, dir string, matches []string, canMatchFiles bool) []string {
	dirs, err := fs.ReadDir(fsys, dir)
	if err != nil {
		// ignore IO errors
		return matches
	}

	// `**` can match *this* dir, so add it
	matches = append(matches, dir)
	for _, info := range dirs {
		name := info.Name()
		if isDir(fsys, dir, name, info) {
			matches = globDoubleStar(fsys, path.Join(dir, name), matches, canMatchFiles)
		} else if canMatchFiles {
			matches = append(matches, path.Join(dir, name))
		}
	}

	return matches
}

// Returns true if the pattern has a doublestar in the middle of the pattern.
// In this case, GlobWalk is faster because it can get away with less
// allocations. However, Glob has a _very_ slight edge if the pattern ends in
// `**`.
func hasMidDoubleStar(p string) bool {
	// subtract 3: 2 because we want to return false if the pattern ends in `**`
	// (Glob is _very_ slightly faster in that case), and the extra 1 because our
	// loop checks p[i] and p[i+1].
	l := len(p) - 3
	for i := 0; i < l; i++ {
		if p[i] == '\\' {
			// escape next byte
			i++
		} else if p[i] == '*' && p[i+1] == '*' {
			return true
		}
	}
	return false
}

// Returns the index of the first unescaped meta character, or negative 1.
func indexMeta(s string) int {
	var c byte
	l := len(s)
	for i := 0; i < l; i++ {
		c = s[i]
		if c == '*' || c == '?' || c == '[' || c == '{' {
			return i
		} else if c == '\\' {
			// skip next byte
			i++
		}
	}
	return -1
}

// Returns the index of the last unescaped slash or closing alt (`}`) in the
// string, or negative 1.
func lastIndexSlashOrAlt(s string) int {
	for i := len(s) - 1; i >= 0; i-- {
		if (s[i] == '/' || s[i] == '}') && (i == 0 || s[i-1] != '\\') {
			return i
		}
	}
	return -1
}

// Returns the index of the last unescaped slash in the string, or negative 1.
func lastIndexSlash(s string) int {
	for i := len(s) - 1; i >= 0; i-- {
		if s[i] == '/' && (i == 0 || s[i-1] != '\\') {
			return i
		}
	}
	return -1
}

// Assuming the byte after the end of `s` is a closing `}`, this function will
// find the index of the matching `{`. That is, it'll skip over any nested `{}`
// and account for escaping.
func indexMatchedOpeningAlt(s string) int {
	alts := 1
	for i := len(s) - 1; i >= 0; i-- {
		if s[i] == '}' && (i == 0 || s[i-1] != '\\') {
			alts++
		} else if s[i] == '{' && (i == 0 || s[i-1] != '\\') {
			if alts--; alts == 0 {
				return i
			}
		}
	}
	return -1
}

// Returns true if the path exists
func exists(fsys fs.FS, name string) bool {
	if _, err := fs.Stat(fsys, name); err != nil {
		return false
	}
	return true
}

// Returns true if the path is a directory, or a symlink to a directory
func isPathDir(fsys fs.FS, name string) bool {
	info, err := fs.Stat(fsys, name)
	if err != nil {
		return false
	}
	return info.IsDir()
}

// Returns whether or not the given DirEntry is a directory. If the DirEntry
// represents a symbolic link, return false
func isDir(fsys fs.FS, dir string, name string, info fs.DirEntry) bool {
	if (info.Type() & fs.ModeSymlink) > 0 {
		return false
	}
	return info.IsDir()
}

// Builds a string from an alt
func buildAlt(prefix, pattern string, startIdx, openingIdx, currentIdx, nextIdx, afterIdx int) string {
	// pattern:
	//   ignored/start{alts,go,here}remaining - len = 36
	//           |    |     | |     ^--- afterIdx   = 27
	//           |    |     | \--------- nextIdx    = 21
	//           |    |     \----------- currentIdx = 19
	//           |    \----------------- openingIdx = 13
	//           \---------------------- startIdx   = 8
	//
	// result:
	//   prefix/startgoremaining - len = 7 + 5 + 2 + 9 = 23
	var buf []byte
	patLen := len(pattern)
	size := (openingIdx - startIdx) + (nextIdx - currentIdx) + (patLen - afterIdx)
	if prefix != "" {
		buf = make([]byte, 0, size+len(prefix)+1)
		buf = append(buf, prefix...)
		buf = append(buf, '/')
	} else {
		buf = make([]byte, 0, size)
	}
	buf = append(buf, pattern[startIdx:openingIdx]...)
	buf = append(buf, pattern[currentIdx:nextIdx]...)
	if afterIdx < patLen {
		buf = append(buf, pattern[afterIdx:]...)
	}
	return string(buf)
}

// Running alts can produce results that are not sorted, and, worse, can cause
// duplicates (consider the trivial pattern `path/to/{a,*}`). Since we know
// each run of doGlob is sorted, we can basically do the "merge" step of a
// merge sort in-place.
func sortAndRemoveDups(matches []string, idx1, idx2, l int) []string {
	var tmp string
	for ; idx1 < idx2; idx1++ {
		if matches[idx1] < matches[idx2] {
			// order is correct
			continue
		} else if matches[idx1] > matches[idx2] {
			// need to swap and then re-sort matches above idx2
			tmp = matches[idx1]
			matches[idx1] = matches[idx2]

			shft := idx2 + 1
			for ; shft < l && matches[shft] < tmp; shft++ {
				matches[shft-1] = matches[shft]
			}
			matches[shft-1] = tmp
		} else {
			// duplicate - shift matches above idx2 down one and decrement l
			for shft := idx2 + 1; shft < l; shft++ {
				matches[shft-1] = matches[shft]
			}
			if l--; idx2 == l {
				// nothing left to do... matches[idx2:] must have been full of dups
				break
			}
		}
	}
	return matches[:l]
}
