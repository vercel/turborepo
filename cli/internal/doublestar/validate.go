// Package doublestar is adapted from https://github.com/bmatcuk/doublestar
// Copyright Bob Matcuk. All Rights Reserved.
// SPDX-License-Identifier: MIT
package doublestar

func doValidatePattern(s string, separator rune) bool {
	altDepth := 0
	l := len(s)
VALIDATE:
	for i := 0; i < l; i++ {
		switch s[i] {
		case '\\':
			if separator != '\\' {
				// skip the next byte - return false if there is no next byte
				if i++; i >= l {
					return false
				}
			}
			continue

		case '[':
			if i++; i >= l {
				// class didn't end
				return false
			}
			if s[i] == '^' || s[i] == '!' {
				i++
			}
			if i >= l || s[i] == ']' {
				// class didn't end or empty character class
				return false
			}

			for ; i < l; i++ {
				if separator != '\\' && s[i] == '\\' {
					i++
				} else if s[i] == ']' {
					// looks good
					continue VALIDATE
				}
			}

			// class didn't end
			return false

		case '{':
			altDepth++
			continue

		case '}':
			if altDepth == 0 {
				// alt end without a corresponding start
				return false
			}
			altDepth--
			continue
		}
	}

	// valid as long as all alts are closed
	return altDepth == 0
}
