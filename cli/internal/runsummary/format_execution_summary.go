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

	if summary.ExecutionSummary.cached == summary.ExecutionSummary.attempted && summary.ExecutionSummary.attempted > 0 {
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

	if summary.ExecutionSummary.attempted == 0 {
		ui.Output("") // Clear the line
		ui.Warn("No tasks were executed as part of this run.")
	}

	ui.Output("") // Clear the line
	ui.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", summary.ExecutionSummary.cached+summary.ExecutionSummary.success, summary.ExecutionSummary.attempted))
	ui.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total${RESET}", summary.ExecutionSummary.cached, summary.ExecutionSummary.attempted))
	ui.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", time.Since(summary.ExecutionSummary.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
	ui.Output("")
}
