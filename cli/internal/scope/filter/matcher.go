package filter

import (
	"regexp"
	"strings"

	"github.com/pkg/errors"
)

type Matcher = func(pkgName string) bool

func matchAll(pkgName string) bool {
	return true
}

func matcherFromPattern(pattern string) (Matcher, error) {
	if pattern == "*" {
		return matchAll, nil
	}

	escaped := regexp.QuoteMeta(pattern)
	// replace escaped '*' with regex '.*'
	normalized := strings.ReplaceAll(escaped, "\\*", ".*")
	if normalized == pattern {
		return func(pkgName string) bool { return pkgName == pattern }, nil
	}
	regex, err := regexp.Compile("^" + normalized + "$")
	if err != nil {
		return nil, errors.Wrapf(err, "failed to compile filter pattern to regex: %v", pattern)
	}
	return func(pkgName string) bool { return regex.Match([]byte(pkgName)) }, nil
}
