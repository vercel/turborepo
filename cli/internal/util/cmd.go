package util

import (
	"bytes"

	"github.com/spf13/cobra"
)

// HelpForCobraCmd returns the help string for a given command
// Note that this overwrites the output for the command
func HelpForCobraCmd(cmd *cobra.Command) string {
	f := cmd.HelpFunc()
	buf := bytes.NewBufferString("")
	cmd.SetOut(buf)
	f(cmd, []string{})
	return buf.String()
}
