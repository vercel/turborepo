//go:build rust
// +build rust

package run

import "github.com/hashicorp/go-hclog"

// LogTag logs out the build tag (in this case "rust") for the current build.
func LogTag(logger hclog.Logger) {
	logger.Debug("build tag: rust")
}
