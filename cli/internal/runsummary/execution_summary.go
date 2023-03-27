package runsummary

import (
	"encoding/json"
	"fmt"
	"os"
	"sync"
	"time"

	"github.com/vercel/turbo/cli/internal/chrometracing"
	"github.com/vercel/turbo/cli/internal/fs"

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

	exitCode *int
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

func (en executionEventName) toString() string {
	switch en {
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

// TaskExecutionSummary contains data about the state of a single task in a turbo run.
// Some fields are updated over time as the task prepares to execute and finishes execution.
type TaskExecutionSummary struct {
	startAt  time.Time          // set once
	status   executionEventName // current status, updated during execution
	err      error              // only populated for failure statuses
	Duration time.Duration      // updated during the task execution
	exitCode *int               // pointer so we can distinguish between 0 and unknown.
}

// MarshalJSON munges the TaskExecutionSummary into a format we want
// We'll use an anonmyous, private struct for this, so it's not confusingly duplicated
func (ts *TaskExecutionSummary) MarshalJSON() ([]byte, error) {
	serializable := struct {
		Start    int64  `json:"startTime"`
		End      int64  `json:"endTime"`
		Status   string `json:"status"`
		Err      error  `json:"error"`
		ExitCode *int   `json:"exitCode"`
	}{
		Start:    ts.startAt.UnixMilli(),
		End:      ts.startAt.Add(ts.Duration).UnixMilli(),
		Status:   ts.status.toString(),
		Err:      ts.err,
		ExitCode: ts.exitCode,
	}

	return json.Marshal(&serializable)
}

// ExitCode access exit code nil means no exit code was received
func (ts *TaskExecutionSummary) ExitCode() *int {
	var exitCode int
	if ts.exitCode == nil {
		return nil
	}
	exitCode = *ts.exitCode
	return &exitCode
}

// executionSummary is the state of the entire `turbo run`. Individual task state in `Tasks` field
type executionSummary struct {
	// mu guards reads/writes to the `state` field
	mu              sync.Mutex
	tasks           map[string]*TaskExecutionSummary // key is a taskID
	profileFilename string

	// These get serialized to JSON
	success   int
	failure   int
	cached    int
	attempted int
	startedAt time.Time
	endedAt   time.Time
	exitCode  int
}

// MarshalJSON munges the executionSummary into a format we want
// We'll use an anonmyous, private struct for this, so it's not confusingly duplicated.
func (es *executionSummary) MarshalJSON() ([]byte, error) {
	serializable := struct {
		Success   int   `json:"success"`
		Failure   int   `json:"failed"`
		Cached    int   `json:"cached"`
		Attempted int   `json:"attempted"`
		StartTime int64 `json:"startTime"`
		EndTime   int64 `json:"endTime"`
		ExitCode  int   `json:"exitCode"`
	}{
		StartTime: es.startedAt.UnixMilli(),
		EndTime:   es.endedAt.UnixMilli(),
		Success:   es.success,
		Failure:   es.failure,
		Cached:    es.cached,
		Attempted: es.attempted,
		ExitCode:  es.exitCode,
	}

	return json.Marshal(&serializable)
}

// newExecutionSummary creates a executionSummary instance to track events in a `turbo run`.`
func newExecutionSummary(start time.Time, tracingProfile string) *executionSummary {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}

	return &executionSummary{
		success:         0,
		failure:         0,
		cached:          0,
		attempted:       0,
		tasks:           make(map[string]*TaskExecutionSummary),
		startedAt:       start,
		profileFilename: tracingProfile,
	}
}

// Run starts the Execution of a single task. It returns a function that can
// be used to update the state of a given taskID with the executionEventName enum
func (es *executionSummary) run(taskID string) (func(outcome executionEventName, err error, exitCode *int), *TaskExecutionSummary) {
	start := time.Now()
	taskExecutionSummary := es.add(&executionEvent{
		Time:   start,
		Label:  taskID,
		Status: targetBuilding,
	})

	tracer := chrometracing.Event(taskID)

	// This function can be called with an enum and an optional error to update
	// the state of a given taskID.
	tracerFn := func(outcome executionEventName, err error, exitCode *int) {
		defer tracer.Done()
		now := time.Now()
		result := &executionEvent{
			Time:     now,
			Duration: now.Sub(start),
			Label:    taskID,
			Status:   outcome,
			// We'll assign this here regardless of whether it is nil, but we'll check for nil
			// when we assign it to the taskExecutionSummary.
			exitCode: exitCode,
		}
		if err != nil {
			result.Err = fmt.Errorf("running %v failed: %w", taskID, err)
		}

		// Ignore the return value here
		es.add(result)
	}

	return tracerFn, taskExecutionSummary
}

func (es *executionSummary) add(event *executionEvent) *TaskExecutionSummary {
	es.mu.Lock()
	defer es.mu.Unlock()

	var taskExecSummary *TaskExecutionSummary
	if ts, ok := es.tasks[event.Label]; ok {
		// If we already know about this task, we'll update it with the new event
		taskExecSummary = ts
	} else {
		// If we don't know about it yet, init and add it into the parent struct
		// (event.Status should always be `targetBuilding` here.)
		taskExecSummary = &TaskExecutionSummary{startAt: event.Time}
		es.tasks[event.Label] = taskExecSummary
	}

	// Update the Status, Duration, and Err fields
	taskExecSummary.status = event.Status
	taskExecSummary.err = event.Err
	taskExecSummary.Duration = event.Duration

	if event.exitCode != nil {
		taskExecSummary.exitCode = event.exitCode
	}

	switch {
	case event.Status == TargetBuildFailed:
		es.failure++
		es.attempted++
	case event.Status == TargetCached:
		es.cached++
		es.attempted++
	case event.Status == TargetBuilt:
		es.success++
		es.attempted++
	}

	return es.tasks[event.Label]
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
