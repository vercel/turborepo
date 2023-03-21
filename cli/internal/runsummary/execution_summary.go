package runsummary

import (
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
	StartAt time.Time `json:"start"`

	Duration time.Duration `json:"duration"`

	// Target which has just changed
	Label string `json:"-"`

	// Its current status
	Status string `json:"status"`

	// Error, only populated for failure statuses
	Err error `json:"error"`
}

// executionSummary is the state of the entire `turbo run`. Individual task state in `Tasks` field
type executionSummary struct {
	// mu guards reads/writes to the `state` field
	mu        sync.Mutex                       `json:"-"`
	state     map[string]*TaskExecutionSummary `json:"-"` // key is a taskID
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
		state:           make(map[string]*TaskExecutionSummary),
		startedAt:       start,
		profileFilename: tracingProfile,
	}
}

// Run starts the Execution of a single task. It returns a function that can
// be used to update the state of a given taskID with the executionEventName enum
func (es *executionSummary) run(label string) (func(outcome executionEventName, err error), *TaskExecutionSummary) {
	start := time.Now()
	taskExecutionSummary := es.add(&executionEvent{
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
		es.add(result)
	}

	return tracerFn, taskExecutionSummary
}

func (es *executionSummary) add(event *executionEvent) *TaskExecutionSummary {
	es.mu.Lock()
	defer es.mu.Unlock()
	if s, ok := es.state[event.Label]; ok {
		s.Status = event.Status.toString()
		s.Err = event.Err
		s.Duration = event.Duration
	} else {
		es.state[event.Label] = &TaskExecutionSummary{
			StartAt:  event.Time,
			Label:    event.Label,
			Status:   event.Status.toString(),
			Err:      event.Err,
			Duration: event.Duration,
		}
	}
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

	return es.state[event.Label]
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
