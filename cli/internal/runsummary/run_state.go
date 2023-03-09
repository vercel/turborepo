package runsummary

import (
	"fmt"
	"os"
	"sync"
	"time"

	"github.com/vercel/turbo/cli/internal/chrometracing"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

// executionEvent represents a single event in the build process, i.e. a target starting or finishing
// building, or reaching some milestone within those steps.
type executionEvent struct {
	// Timestamp of this event
	Time time.Time
	// Duration of this event
	Duration time.Duration
	// Target which has just changed
	Label string
	// Its current status
	Status executionEventName
	// Error, only populated for failure statuses
	Err error
}

// executionEventName represents the status of a target when we log a build result.
type executionEventName int

// The collection of expected build result statuses.
const (
	targetBuilding executionEventName = iota
	TargetBuildStopped
	TargetBuilt
	TargetCached
	TargetBuildFailed
)

func (rrs executionEventName) toString() string {
	switch rrs {
	case targetBuilding:
		return "building"
	case TargetBuildStopped:
		return "buildStopped"
	case TargetBuilt:
		return "built"
	case TargetCached:
		return "cached"
	case TargetBuildFailed:
		return "buildFailed"
	}

	return ""
}

// runState is the state of the entire `turbo run`. Individual task state in `Tasks` field
// TODO(mehulkar): Can this be combined with the RunSummary?
type runState struct {
	mu      sync.Mutex
	state   map[string]*TaskExecutionSummary
	success int
	failure int
	// Is the output streaming?
	cached    int
	attempted int

	startedAt time.Time

	profileFilename string
}

// newRunState creates a runState instance to track events in a `turbo run`.`
func newRunState(start time.Time, tracingProfile string) *runState {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}

	return &runState{
		success:         0,
		failure:         0,
		cached:          0,
		attempted:       0,
		state:           make(map[string]*TaskExecutionSummary),
		startedAt:       start,
		profileFilename: tracingProfile,
	}
}

// Run starts the Execution of a single task. It returns a function that can
// be used to update the state of a given taskID with the executionEventName enum
func (r *runState) run(label string) (func(outcome executionEventName, err error), *TaskExecutionSummary) {
	start := time.Now()
	taskExecutionSummary := r.add(&executionEvent{
		Time:   start,
		Label:  label,
		Status: targetBuilding,
	})

	tracer := chrometracing.Event(label)

	// This function can be called with an enum and an optional error to update
	// the state of a given taskID.
	tracerFn := func(outcome executionEventName, err error) {
		defer tracer.Done()
		now := time.Now()
		result := &executionEvent{
			Time:     now,
			Duration: now.Sub(start),
			Label:    label,
			Status:   outcome,
		}
		if err != nil {
			result.Err = fmt.Errorf("running %v failed: %w", label, err)
		}
		// Ignore the return value here
		r.add(result)
	}

	return tracerFn, taskExecutionSummary
}

func (r *runState) add(result *executionEvent) *TaskExecutionSummary {
	r.mu.Lock()
	defer r.mu.Unlock()
	if s, ok := r.state[result.Label]; ok {
		s.Status = result.Status.toString()
		s.Err = result.Err
		s.Duration = result.Duration
	} else {
		r.state[result.Label] = &TaskExecutionSummary{
			StartAt:  result.Time,
			Label:    result.Label,
			Status:   result.Status.toString(),
			Err:      result.Err,
			Duration: result.Duration,
		}
	}
	switch {
	case result.Status == TargetBuildFailed:
		r.failure++
		r.attempted++
	case result.Status == TargetCached:
		r.cached++
		r.attempted++
	case result.Status == TargetBuilt:
		r.success++
		r.attempted++
	}

	return r.state[result.Label]
}

// Close finishes a trace of a turbo run. The tracing file will be written if applicable,
// and run stats are written to the terminal
func (r *runState) close(terminal cli.Ui) error {
	if err := writeChrometracing(r.profileFilename, terminal); err != nil {
		terminal.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	maybeFullTurbo := ""
	if r.cached == r.attempted && r.attempted > 0 {
		terminalProgram := os.Getenv("TERM_PROGRAM")
		// On the macOS Terminal, the rainbow colors show up as a magenta background
		// with a gray background on a single letter. Instead, we print in bold magenta
		if terminalProgram == "Apple_Terminal" {
			fallbackTurboColor := color.New(color.FgHiMagenta, color.Bold).SprintFunc()
			maybeFullTurbo = fallbackTurboColor(">>> FULL TURBO")
		} else {
			maybeFullTurbo = ui.Rainbow(">>> FULL TURBO")
		}
	}

	if r.attempted == 0 {
		terminal.Output("") // Clear the line
		terminal.Warn("No tasks were executed as part of this run.")
	}
	terminal.Output("") // Clear the line
	terminal.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", r.cached+r.success, r.attempted))
	terminal.Output(util.Sprintf("${BOLD}cached:    %v cached${RESET}${GRAY}, %v total${RESET}", r.cached, r.attempted))
	terminal.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", time.Since(r.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
	terminal.Output("")
	return nil
}

// writeChromeTracing writes to a profile name if the `--profile` flag was passed to turbo run
func writeChrometracing(filename string, terminal cli.Ui) error {
	outputPath := chrometracing.Path()
	if outputPath == "" {
		// tracing wasn't enabled
		return nil
	}

	name := fmt.Sprintf("turbo-%s.trace", time.Now().Format(time.RFC3339))
	if filename != "" {
		name = filename
	}
	if err := chrometracing.Close(); err != nil {
		terminal.Warn(fmt.Sprintf("Failed to flush tracing data: %v", err))
	}
	cwdRaw, err := os.Getwd()
	if err != nil {
		return err
	}
	root, err := fs.GetCwd(cwdRaw)
	if err != nil {
		return err
	}
	// chrometracing.Path() is absolute by default, but can still be relative if overriden via $CHROMETRACING_DIR
	// so we have to account for that before converting to turbopath.AbsoluteSystemPath
	if err := fs.CopyFile(&fs.LstatCachedFile{Path: fs.ResolveUnknownPath(root, outputPath)}, name); err != nil {
		return err
	}
	return nil
}
