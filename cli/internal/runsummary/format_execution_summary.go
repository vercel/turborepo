package runsummary

import (
	"fmt"
	"os"
	"sort"
	"strings"
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
	failed := rsm.RunSummary.getFailedTasks() // Note: ExecutionSummary.failure exists, but we need the task names
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

	lineData := []summaryLine{
		{header: "Tasks", trailer: util.Sprintf("${BOLD_GREEN}%v successful${RESET}${GRAY}, %v total", successful, attempted)},
		{header: "Cached", trailer: util.Sprintf("%v cached${RESET}${GRAY}, %v total", cached, attempted)},
		{header: "Time", trailer: util.Sprintf("%v${RESET} %v", duration, maybeFullTurbo)},
	}

	if rsm.getPath().FileExists() {
		l := summaryLine{header: "Summary", trailer: util.Sprintf("%s", rsm.getPath())}
		lineData = append(lineData, l)
	}

	if len(failed) > 0 {
		formatted := []string{}
		for _, t := range failed {
			formatted = append(formatted, util.Sprintf("${BOLD_RED}%s${RESET}", t.TaskID))
		}
		sort.Strings(formatted) // To make the order deterministic
		l := summaryLine{header: "Failed", trailer: strings.Join(formatted, ", ")}
		lineData = append(lineData, l)
	}

	// Some info we need for left padding
	maxlength := 0
	for _, sl := range lineData {
		if len(sl.header) > maxlength {
			maxlength = len(sl.header)
		}
	}

	lines := []string{}
	for _, sl := range lineData {
		paddedHeader := fmt.Sprintf("%*s", maxlength, sl.header)
		line := util.Sprintf("${BOLD}%s:    %s${RESET}", paddedHeader, sl.trailer)
		lines = append(lines, line)
	}

	// Print the lines to terminal
	if attempted == 0 {
		ui.Output("") // Clear the line
		ui.Warn("No tasks were executed as part of this run.")
	}

	ui.Output("") // Clear the line

	for _, line := range lines {
		ui.Output(line)
	}

	ui.Output("")
}

type summaryLine struct {
	header  string
	trailer string
}
