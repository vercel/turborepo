// Package doublestar is adapted from https://github.com/bmatcuk/doublestar
// Copyright Bob Matcuk. All Rights Reserved.
// SPDX-License-Identifier: MIT
package doublestar

// SplitPattern is a utility function. Given a pattern, SplitPattern will
// return two strings: the first string is everything up to the last slash
// (`/`) that appears _before_ any unescaped "meta" characters (ie, `*?[{`).
// The second string is everything after that slash. For example, given the
// pattern:
//
//	../../path/to/meta*/**
//	             ^----------- split here
//
// SplitPattern returns "../../path/to" and "meta*/**". This is useful for
// initializing os.DirFS() to call Glob() because Glob() will silently fail if
// your pattern includes `/./` or `/../`. For example:
//
//	base, pattern := SplitPattern("../../path/to/meta*/**")
//	fsys := os.DirFS(base)
//	matches, err := Glob(fsys, pattern)
//
// If SplitPattern cannot find somewhere to split the pattern (for example,
// `meta*/**`), it will return "." and the unaltered pattern (`meta*/**` in
// this example).
//
// Of course, it is your responsibility to decide if the returned base path is
// "safe" in the context of your application. Perhaps you could use Match() to
// validate against a list of approved base directories?
func SplitPattern(p string) (string, string) {
	base := "."
	pattern := p

	splitIdx := -1
	for i := 0; i < len(p); i++ {
		c := p[i]
		if c == '\\' {
			i++
		} else if c == '/' {
			splitIdx = i
		} else if c == '*' || c == '?' || c == '[' || c == '{' {
			break
		}
	}

	if splitIdx >= 0 {
		return p[:splitIdx], p[splitIdx+1:]
	}

	return base, pattern
}

// Finds the next comma, but ignores any commas that appear inside nested `{}`.
// Assumes that each opening bracket has a corresponding closing bracket.
func indexNextAlt(s string, allowEscaping bool) int {
	alts := 1
	l := len(s)
	for i := 0; i < l; i++ {
		if allowEscaping && s[i] == '\\' {
			// skip next byte
			i++
		} else if s[i] == '{' {
			alts++
		} else if s[i] == '}' {
			alts--
		} else if s[i] == ',' && alts == 1 {
			return i
		}
	}
	return -1
}
