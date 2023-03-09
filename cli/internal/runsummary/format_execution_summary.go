package runsummary

import (
	"os"
	"time"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
	internalUI "github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
)

func (summary *RunSummary) printExecutionSummary(ui cli.Ui) {
	maybeFullTurbo := ""
	if summary.ExecutionSummary.Cached == summary.ExecutionSummary.Attempted && summary.ExecutionSummary.Attempted > 0 {
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

	if summary.ExecutionSummary.Attempted == 0 {
		ui.Output("") // Clear the line
		ui.Warn("No tasks were executed as part of this run.")
	}
	ui.Output("") // Clear the line
	ui.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", summary.ExecutionSummary.Cached+summary.ExecutionSummary.Success, summary.ExecutionSummary.Attempted))
	ui.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total${RESET}", summary.ExecutionSummary.Cached, summary.ExecutionSummary.Attempted))
	ui.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", time.Since(summary.ExecutionSummary.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
	ui.Output("")
}
