package runsummary

import (
	"os"
	"time"

	"github.com/fatih/color"
	internalUI "github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
)

func (rsm *Meta) printExecutionSummary() {
	maybeFullTurbo := ""
	summary := rsm.RunSummary
	ui := rsm.ui

	attempted := summary.ExecutionSummary.attempted
	successful := summary.ExecutionSummary.cached + summary.ExecutionSummary.success
	cached := summary.ExecutionSummary.cached
	// TODO: can we use a method on ExecutionSummary here?
	duration := time.Since(summary.ExecutionSummary.startedAt).Truncate(time.Millisecond)

	if cached == attempted && attempted > 0 {
		terminalProgram := os.Getenv("TERM_PROGRAM")
		// On the macOS Terminal, the rainbow colors show up as a magenta background
		// with a gray background on a single letter. Instead, we print in bold magenta
		if terminalProgram == "Apple_Terminal" {
			fallbackTurboColor := color.New(color.FgHiMagenta, color.Bold).SprintFunc()
			maybeFullTurbo = fallbackTurboColor(">>> FULL TURBO")
		} else {
			maybeFullTurbo = internalUI.Rainbow(">>> FULL TURBO")
		}
	}

	if attempted == 0 {
		ui.Output("") // Clear the line
		ui.Warn("No tasks were executed as part of this run.")
	}

	ui.Output("") // Clear the line

	// The whitespaces in the string are to align the UI in a nice way.
	ui.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", successful, attempted))
	ui.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total${RESET}", cached, attempted))
	ui.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", duration, maybeFullTurbo))
	ui.Output("")
}
