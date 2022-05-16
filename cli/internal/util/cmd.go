package util

import (
	"bytes"

	"github.com/spf13/cobra"
)

// ExitCodeError is a specific error that is returned by the command to specify the exit code
type ExitCodeError struct {
	ExitCode int
}

func (e *ExitCodeError) Error() string { return "exit code error" }

// HelpForCobraCmd returns the help string for a given command
// Note that this overwrites the output for the command
func HelpForCobraCmd(cmd *cobra.Command) string {
	f := cmd.HelpFunc()
	buf := bytes.NewBufferString("")
	cmd.SetOut(buf)
	f(cmd, []string{})
	return buf.String()
}
