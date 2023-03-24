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
	duration time.Duration      // updated during the task execution
}

// MarshalJSON munges the TaskExecutionSummary into a format we want
// We'll use an anonmyous, private struct for this, so it's not confusingly duplicated
func (ts *TaskExecutionSummary) MarshalJSON() ([]byte, error) {
	serializable := struct {
		Start  int64  `json:"startTime"`
		End    int64  `json:"endTime"`
		Status string `json:"status"`
		Err    error  `json:"error"`
	}{
		Start:  ts.startAt.UnixMilli(),
		End:    ts.startAt.Add(ts.duration).UnixMilli(),
		Status: ts.status.toString(),
		Err:    ts.err,
	}

	return json.Marshal(&serializable)
}

// executionSummary is the state of the entire `turbo run`. Individual task state in `Tasks` field
type executionSummary struct {
	// mu guards reads/writes to the `state` field
	mu        sync.Mutex                       `json:"-"`
	tasks     map[string]*TaskExecutionSummary `json:"-"` // key is a taskID
	Success   int                              `json:"success"`
	Failure   int                              `json:"failed"`
	Cached    int                              `json:"cached"`
	Attempted int                              `json:"attempted"`

	startedAt time.Time

	profileFilename string
}

// newExecutionSummary creates a executionSummary instance to track events in a `turbo run`.`
func newExecutionSummary(start time.Time, tracingProfile string) *executionSummary {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}

	return &executionSummary{
		Success:         0,
		Failure:         0,
		Cached:          0,
		Attempted:       0,
		tasks:           make(map[string]*TaskExecutionSummary),
		startedAt:       start,
		profileFilename: tracingProfile,
	}
}

// Run starts the Execution of a single task. It returns a function that can
// be used to update the state of a given taskID with the executionEventName enum
func (es *executionSummary) run(taskID string) (func(outcome executionEventName, err error), *TaskExecutionSummary) {
	start := time.Now()
	taskExecutionSummary := es.add(&executionEvent{
		Time:   start,
		Label:  taskID,
		Status: targetBuilding,
	})

	tracer := chrometracing.Event(taskID)

	// This function can be called with an enum and an optional error to update
	// the state of a given taskID.
	tracerFn := func(outcome executionEventName, err error) {
		defer tracer.Done()
		now := time.Now()
		result := &executionEvent{
			Time:     now,
			Duration: now.Sub(start),
			Label:    taskID,
			Status:   outcome,
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
	taskExecSummary.duration = event.Duration

	switch {
	case event.Status == TargetBuildFailed:
		es.Failure++
		es.Attempted++
	case event.Status == TargetCached:
		es.Cached++
		es.Attempted++
	case event.Status == TargetBuilt:
		es.Success++
		es.Attempted++
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
