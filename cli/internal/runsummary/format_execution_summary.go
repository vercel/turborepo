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

	var lines []string

	// The only difference between these two branches is that when there is a run summary
	// we print the path to that file and we adjust the whitespace in the printed text so it aligns.
	// We could just always align to account for the summary line, but that would require a whole
	// bunch of test output assertions to change.
	if rsm.getPath().FileExists() {
		lines = []string{
			util.Sprintf("${BOLD}  Tasks:${BOLD_GREEN}%s%v successful${RESET}${GRAY}, %v total${RESET}", spacer, successful, attempted),
			util.Sprintf("${BOLD} Cached:%s%v cached${RESET}${GRAY}, %v total${RESET}", spacer, cached, attempted),
			util.Sprintf("${BOLD}   Time:%s%v${RESET} %v${RESET}", spacer, duration, maybeFullTurbo),
			util.Sprintf("${BOLD}Summary:%s%s${RESET}", spacer, rsm.getPath()),
		}
	} else {
		lines = []string{
			util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}%s%v successful${RESET}${GRAY}, %v total${RESET}", spacer, successful, attempted),
			util.Sprintf("${BOLD}Cached:%s%v cached${RESET}${GRAY}, %v total${RESET}", spacer, cached, attempted),
			util.Sprintf("${BOLD}  Time:%s%v${RESET} %v${RESET}", spacer, duration, maybeFullTurbo),
		}
	}

	// Print the real thing
	for _, line := range lines {
		ui.Output(line)
	}

	ui.Output("")
}
