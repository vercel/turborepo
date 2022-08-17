package run

import (
	"encoding/json"
	"fmt"
	"sync"
	"time"

	"github.com/segmentio/ksuid"
	"github.com/vercel/turborepo/cli/internal/chrometracing"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/mitchellh/cli"
)

// RunResult represents a single event in the build process, i.e. a target starting or finishing
// building, or reaching some milestone within those steps.
type RunResult struct {
	// Timestamp of this event
	Time time.Time
	// Duration of this event
	Duration time.Duration
	// Target which has just changed
	Label string
	// Its current status
	Status RunResultStatus
	// Error, only populated for failure statuses
	Err error
}

// RunResultStatus represents the status of a target when we log a build result.
type RunResultStatus int

// The collection of expected build result statuses.
const (
	TargetBuilding RunResultStatus = iota
	TargetBuildStopped
	TargetBuilt
	TargetCached
	TargetBuildFailed
	TargetNonexistent
)

func (rr RunResultStatus) String() string {
	switch rr {
	case TargetBuilding:
		return "running"
	case TargetBuildStopped:
		return "stopped"
	case TargetBuilt:
		return "executed"
	case TargetCached:
		return "replayed"
	case TargetBuildFailed:
		return "failed"
	case TargetNonexistent:
		return "nonexistent"
	default:
		panic(fmt.Sprintf("unknown status: %v", int(rr)))
	}
}

type BuildTargetState struct {
	StartAt time.Time

	Duration time.Duration
	// Target which has just changed
	Label string
	// Its current status
	Status RunResultStatus
	// Error, only populated for failure statuses
	Err error
}

type RunState struct {
	sessionID ksuid.KSUID
	mu        sync.Mutex
	state     map[string]*BuildTargetState
	Success   int
	Failure   int
	// Is the output streaming?
	Cached    int
	Attempted int

	startedAt time.Time
}

// NewRunState creates a RunState instance for tracking events during the
// course of a run.
func NewRunState(startedAt time.Time, tracingProfile string, sessionID ksuid.KSUID) *RunState {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}
	return &RunState{
		sessionID: sessionID,
		Success:   0,
		Failure:   0,
		Cached:    0,
		Attempted: 0,
		state:     make(map[string]*BuildTargetState),

		startedAt: startedAt,
	}
}

func (r *RunState) Run(label string) func(outcome RunResultStatus, err error) {
	start := time.Now()
	r.add(&RunResult{
		Time:   start,
		Label:  label,
		Status: TargetBuilding,
	}, label, true)
	tracer := chrometracing.Event(label)
	return func(outcome RunResultStatus, err error) {
		defer tracer.Done()
		now := time.Now()
		result := &RunResult{
			Time:     now,
			Duration: now.Sub(start),
			Label:    label,
			Status:   outcome,
		}
		if err != nil {
			result.Err = fmt.Errorf("running %v failed: %w", label, err)
		}
		r.add(result, label, false)
	}
}

func (r *RunState) add(result *RunResult, previous string, active bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if s, ok := r.state[result.Label]; ok {
		s.Status = result.Status
		s.Err = result.Err
		s.Duration = result.Duration
	} else {
		r.state[result.Label] = &BuildTargetState{
			StartAt:  result.Time,
			Label:    result.Label,
			Status:   result.Status,
			Err:      result.Err,
			Duration: result.Duration,
		}
	}
	switch {
	case result.Status == TargetBuildFailed:
		r.Failure++
		r.Attempted++
	case result.Status == TargetCached:
		r.Cached++
		r.Attempted++
	case result.Status == TargetBuilt:
		r.Success++
		r.Attempted++
	}
}

// Close finishes a trace of a turbo run. The tracing file will be written if applicable,
// and run stats are written to the terminal
func (r *RunState) Close(terminal cli.Ui, filename string, summaryPath fs.AbsolutePath) error {
	endedAt := time.Now()
	if err := writeChrometracing(filename, terminal); err != nil {
		terminal.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	if err := r.writeSummary(summaryPath, endedAt); err != nil {
		terminal.Error(fmt.Sprintf("Error writing run summary: %v", err))
	}

	maybeFullTurbo := ""
	if r.Cached == r.Attempted && r.Attempted > 0 {
		maybeFullTurbo = ui.Rainbow(">>> FULL TURBO")
	}
	terminal.Output("") // Clear the line
	terminal.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", r.Cached+r.Success, r.Attempted))
	terminal.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total${RESET}", r.Cached, r.Attempted))
	terminal.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", endedAt.Sub(r.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
	terminal.Output("")
	return nil
}

func (r *RunState) writeSummary(summaryPath fs.AbsolutePath, endedAt time.Time) error {
	if err := summaryPath.EnsureDir(); err != nil {
		return err
	}
	summary := make(map[string]interface{})
	summary["sessionId"] = r.sessionID.String()
	summary["startedAt"] = r.startedAt.UnixMilli()
	summary["endedAt"] = endedAt.UnixMilli()
	summary["durationMs"] = endedAt.Sub(r.startedAt).Milliseconds()
	tasks := make(map[string]interface{})
	for task, targetState := range r.state {
		taskSummary := make(map[string]interface{})
		taskSummary["startedAt"] = targetState.StartAt.UnixMilli()
		taskSummary["endedAt"] = targetState.StartAt.Add(targetState.Duration).UnixMilli()
		taskSummary["durationMs"] = targetState.Duration.Milliseconds()
		taskSummary["status"] = targetState.Status.String()
		if targetState.Err != nil {
			taskSummary["error"] = targetState.Err.Error()
		}
		tasks[task] = taskSummary
	}
	summary["tasks"] = tasks
	bytes, err := json.MarshalIndent(summary, "", "\t")
	if err != nil {
		return err
	}
	if err := summaryPath.WriteFile(bytes, 0644); err != nil {
		return err
	}
	return nil
}

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
	root, err := fs.GetCwd()
	if err != nil {
		return err
	}
	// chrometracing.Path() is absolute by default, but can still be relative if overriden via $CHROMETRACING_DIR
	// so we have to account for that before converting to AbsolutePath
	if err := fs.CopyFile(&fs.LstatCachedFile{Path: fs.ResolveUnknownPath(root, outputPath)}, name); err != nil {
		return err
	}
	return nil
}
