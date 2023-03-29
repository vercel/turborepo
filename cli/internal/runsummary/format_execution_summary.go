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

	ui.Output("")    // Clear the line
	spacer := "    " // 4 chars

	// We'll start with some default lines
	lines := []string{
		util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}%s%v successful${RESET}${GRAY}, %v total${RESET}", spacer, successful, attempted),
		util.Sprintf("${BOLD}Cached:%s%v cached${RESET}${GRAY}, %v total${RESET}", spacer, cached, attempted),
		util.Sprintf("${BOLD}  Time:%s%v${RESET} %v${RESET}", spacer, duration, maybeFullTurbo),
	}

	// If we have a run summary file, add that in. We have to do some whitespace adjusting, so just
	// duplicate the lines above instead of using some fancy tabbibg/spacing mechanism.
	if rsm.getPath().FileExists() {
		lines = []string{
			util.Sprintf("${BOLD}  Tasks:${BOLD_GREEN}%s%v successful${RESET}${GRAY}, %v total${RESET}", spacer, successful, attempted),
			util.Sprintf("${BOLD} Cached:%s%v cached${RESET}${GRAY}, %v total${RESET}", spacer, cached, attempted),
			util.Sprintf("${BOLD}   Time:%s%v${RESET} %v${RESET}", spacer, duration, maybeFullTurbo),
			util.Sprintf("${BOLD}Summary:%s%s${RESET}", spacer, rsm.getPath()),
		}
	}

	// Print the real thing
	for _, line := range lines {
		ui.Output(line)
	}

	ui.Output("")
}
