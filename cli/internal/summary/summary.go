package summary

import (
	"encoding/json"
	"fmt"
	"strings"
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

type taskSummary struct {
	startAt time.Time

	duration time.Duration
	// taskID of the task which has just changed
	taskID string
	// Its current state
	state TaskState
	// Error, only populated for failure statuses
	err error

	cacheResults cacheResult

	hash string
}

// Summary collects information over the course of a turbo run
// to produce a summary
type Summary struct {
	sessionID ksuid.KSUID
	mu        sync.Mutex
	state     map[string]*taskSummary
	targets   []string
	pkgs      []string
	rawArgs   []string
	success   int
	failure   int
	// Is the output streaming?
	cached    int
	attempted int

	startedAt time.Time
}

// New creates a RunState instance for tracking events during the
// course of a run.
func New(startedAt time.Time, tracingProfile string, sessionID ksuid.KSUID, rawArgs []string, pkgs []string, targets []string) *Summary {
	if tracingProfile != "" {
		chrometracing.EnableTracing()
	}
	return &Summary{
		sessionID: sessionID,
		targets:   targets,
		pkgs:      pkgs,
		rawArgs:   rawArgs,
		success:   0,
		failure:   0,
		cached:    0,
		attempted: 0,
		state:     make(map[string]*taskSummary),

		startedAt: startedAt,
	}
}

// Trace is a handle given to a single task so it can record events
type Trace struct {
	summary     *Summary
	chromeEvent *chrometracing.PendingEvent
	taskSummary *taskSummary
}

// AddCacheResults records per-task cache information
func (t *Trace) AddCacheResults(results cacheResult) {
	t.taskSummary.cacheResults = results
}

// SetFailed marks this task as failed with the given error
func (t *Trace) SetFailed(err error) {
	t.taskSummary.err = err
	t.taskSummary.state = TaskStateFailed
}

// SetResult marks the outcome for this task
func (t *Trace) SetResult(state TaskState) {
	t.taskSummary.state = state
}

// SetHash records the hash for this task
func (t *Trace) SetHash(hash string) {
	t.taskSummary.hash = hash
}

// Finish records this task as being finished with the given outcome
func (t *Trace) Finish() {
	t.chromeEvent.Done()
	now := time.Now()
	t.taskSummary.duration = now.Sub(t.summary.startedAt)
	t.summary.add(t.taskSummary)
}

// StartTrace returns a handle to track events for a given task
func (r *Summary) StartTrace(taskID string) *Trace {
	start := time.Now()
	ts := &taskSummary{
		startAt: start,
		taskID:  taskID,
		state:   TaskStateRunning,
	}
	tracer := chrometracing.Event(taskID)
	return &Trace{
		summary:     r,
		taskSummary: ts,
		chromeEvent: tracer,
	}
}

func (r *Summary) add(taskSummary *taskSummary) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.state[taskSummary.taskID] = taskSummary
	switch taskSummary.state {
	case TaskStateFailed:
		r.failure++
		r.attempted++
	case TaskStateCached:
		r.cached++
		r.attempted++
	case TaskStateCompleted:
		r.success++
		r.attempted++
	default:
	}
}

// Close finishes a trace of a turbo run. The tracing file will be written if applicable,
// and run stats are written to the terminal
// TODO: versioning the serialized summary
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
	summary["entrypointPackages"] = r.pkgs
	summary["targets"] = r.targets
	summary["command"] = strings.Join(r.rawArgs, " ")
	summary["durationMs"] = endedAt.Sub(r.startedAt).Milliseconds()
	tasks := make(map[string]interface{})
	for task, targetState := range r.state {
		taskSummary := make(map[string]interface{})
		taskSummary["startedAt"] = targetState.startAt.UnixMilli()
		taskSummary["endedAt"] = targetState.startAt.Add(targetState.duration).UnixMilli()
		taskSummary["durationMs"] = targetState.duration.Milliseconds()
		taskSummary["status"] = targetState.state.String()
		taskSummary["taskHash"] = targetState.hash
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
