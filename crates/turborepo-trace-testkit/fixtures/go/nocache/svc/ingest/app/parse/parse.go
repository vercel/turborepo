package parse

import "strings"

// Fields splits a log line.
func Fields(line string) []string {
	return strings.Fields(line)
}
