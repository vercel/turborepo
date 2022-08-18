package summary

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

type cacheResult interface{}

// taskEvent represents a single event in the build process, i.e. a target starting or finishing
// building, or reaching some milestone within those steps.
type taskEvent struct {
	// Timestamp of this event
	time time.Time
	// duration of this event
	duration time.Duration
	// Target which has just changed
	taskID string
	// Its current status
	taskState TaskState
	// Error, only populated for failure statuses
	err error
}

// TaskState represents the status of a target when we log a build result.
type TaskState int

// The collection of expected build result statuses.
const (
	TaskStateRunning TaskState = iota
	TaskStateStopped
	TaskStateCompleted
	TaskStateCached
	TaskStateFailed
	TaskStateNonexistent
)

func (ts TaskState) String() string {
	switch ts {
	case TaskStateRunning:
		return "running"
	case TaskStateStopped:
		return "stopped"
	case TaskStateCompleted:
		return "executed"
	case TaskStateCached:
		return "replayed"
	case TaskStateFailed:
		return "failed"
	case TaskStateNonexistent:
		return "nonexistent"
	default:
		panic(fmt.Sprintf("unknown status: %v", int(ts)))
	}
}

type taskState struct {
	startAt time.Time

	duration time.Duration
	// taskID of the task which has just changed
	taskID string
	// Its current status
	status TaskState
	// Error, only populated for failure statuses
	err error

	cacheResults cacheResult
}

// Summary collects information over the course of a turbo run
// to produce a summary
type Summary struct {
	sessionID ksuid.KSUID
	mu        sync.Mutex
	state     map[string]*taskState
	success   int
	failure   int
	// Is the output streaming?
	cached    int
	attempted int

	startedAt time.Time
}

// New creates a RunState instance for tracking events during the
// course of a run.
func New(startedAt time.Time, tracingProfile string, sessionID ksuid.KSUID) *Summary {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}
	return &Summary{
		sessionID: sessionID,
		success:   0,
		failure:   0,
		cached:    0,
		attempted: 0,
		state:     make(map[string]*taskState),

		startedAt: startedAt,
	}
}

// Trace is a handle given to a single task so it can record events
type Trace struct {
	taskID      string
	rs          *Summary
	start       time.Time
	chromeEvent *chrometracing.PendingEvent
}

// AddCacheResults records per-task cache information
func (t *Trace) AddCacheResults(results cacheResult) {
	t.rs.addCacheResults(t.taskID, results)
}

// Finish records this task as being finished with the given outcome
func (t *Trace) Finish(outcome TaskState, err error) {
	t.chromeEvent.Done()
	now := time.Now()
	result := &taskEvent{
		time:      now,
		duration:  now.Sub(t.start),
		taskID:    t.taskID,
		taskState: outcome,
	}
	if err != nil {
		result.err = fmt.Errorf("running %v failed: %w", t.taskID, err)
	}
	t.rs.add(result, t.taskID, false)
}

// StartTrace returns a handle to track events for a given task
func (r *Summary) StartTrace(taskID string) *Trace {
	start := time.Now()
	r.add(&taskEvent{
		time:      start,
		taskID:    taskID,
		taskState: TaskStateRunning,
	}, taskID, true)
	tracer := chrometracing.Event(taskID)
	return &Trace{
		taskID:      taskID,
		rs:          r,
		start:       start,
		chromeEvent: tracer,
	}
}

func (r *Summary) addCacheResults(taskID string, cacheResult cacheResult) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if s, ok := r.state[taskID]; ok {
		s.cacheResults = cacheResult
	}
}

func (r *Summary) add(result *taskEvent, previous string, active bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if s, ok := r.state[result.taskID]; ok {
		s.status = result.taskState
		s.err = result.err
		s.duration = result.duration
	} else {
		r.state[result.taskID] = &taskState{
			startAt:  result.time,
			taskID:   result.taskID,
			status:   result.taskState,
			err:      result.err,
			duration: result.duration,
		}
	}
	switch {
	case result.taskState == TaskStateFailed:
		r.failure++
		r.attempted++
	case result.taskState == TaskStateCached:
		r.cached++
		r.attempted++
	case result.taskState == TaskStateCompleted:
		r.success++
		r.attempted++
	}
}

// Close finishes a trace of a turbo run. The tracing file will be written if applicable,
// and run stats are written to the terminal
func (r *Summary) Close(terminal cli.Ui, filename string, summaryPath fs.AbsolutePath) error {
	endedAt := time.Now()
	if err := writeChrometracing(filename, terminal); err != nil {
		terminal.Error(fmt.Sprintf("Error writing tracing data: %v", err))
	}

	if err := r.writeSummary(summaryPath, endedAt); err != nil {
		terminal.Error(fmt.Sprintf("Error writing run summary: %v", err))
	}

	maybeFullTurbo := ""
	if r.cached == r.attempted && r.attempted > 0 {
		maybeFullTurbo = ui.Rainbow(">>> FULL TURBO")
	}
	terminal.Output("") // Clear the line
	terminal.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total${RESET}", r.cached+r.success, r.attempted))
	terminal.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total${RESET}", r.cached, r.attempted))
	terminal.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v${RESET}", endedAt.Sub(r.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
	terminal.Output("")
	return nil
}

func (r *Summary) writeSummary(summaryPath fs.AbsolutePath, endedAt time.Time) error {
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
		taskSummary["startedAt"] = targetState.startAt.UnixMilli()
		taskSummary["endedAt"] = targetState.startAt.Add(targetState.duration).UnixMilli()
		taskSummary["durationMs"] = targetState.duration.Milliseconds()
		taskSummary["status"] = targetState.status.String()
		taskSummary["cache"] = targetState.cacheResults
		if targetState.err != nil {
			taskSummary["error"] = targetState.err.Error()
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
