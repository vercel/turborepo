package run

import (
	"fmt"
	"log"
	"math"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	cursor "github.com/vercel/turborepo/cli/internal/ui/term"

	"github.com/google/chrometracing"
	"github.com/mitchellh/cli"
)

// A RunResult represents a single event in the build process, i.e. a target starting or finishing
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
	// Description of what's going on right now.
	Description string
	// Test results
	// Tests TestSuite
}

// A RunResultStatus represents the status of a target when we log a build result.
type RunResultStatus int

// The collection of expected build result statuses.
const (
	TargetBuilding RunResultStatus = iota
	TargetBuildStopped
	TargetBuilt
	TargetCached
	TargetBuildFailed
	TargetTesting
	TargetTestStopped
	TargetTested
	TargetTestFailed
)

type BuildTargetState struct {
	StartAt time.Time

	Duration time.Duration
	// Target which has just changed
	Label string
	// Its current status
	Status RunResultStatus
	// Error, only populated for failure statuses
	Err error
	// Description of what's going on right now.
	Description string
}

type RunState struct {
	mu      sync.Mutex
	Ordered []string
	state   map[string]*BuildTargetState
	done    chan string
	Success int
	Failure int
	// Is the output streaming?
	Cached     int
	Attempted  int
	Scope      []string
	Changed    []string
	runOptions *RunOptions
	cursor     *cursor.Cursor
	ticker     *time.Ticker

	startedAt time.Time
}

func NewRunState(runOptions *RunOptions, startedAt time.Time) *RunState {
	return &RunState{
		Success:   0,
		Failure:   0,
		Cached:    0,
		Attempted: 0,
		state:     make(map[string]*BuildTargetState),

		cursor:     cursor.New(),
		runOptions: runOptions,
		startedAt:  startedAt,
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
		switch {
		case outcome == TargetBuildFailed:
			r.add(&RunResult{
				Time:        time.Now(),
				Duration:    time.Since(start),
				Label:       label,
				Status:      TargetBuildFailed,
				Err:         fmt.Errorf("running %v failed: %w", label, err),
				Description: fmt.Sprintf("running %v failed", label),
			}, label, false)
		case outcome == TargetCached:
			r.add(&RunResult{
				Time:        time.Now(),
				Duration:    time.Since(start),
				Label:       label,
				Description: label + " cached",
				Status:      TargetCached,
			}, label, false)
		case outcome == TargetBuildStopped:
			r.add(&RunResult{
				Time:        time.Now(),
				Duration:    time.Since(start),
				Label:       label,
				Description: label + " stopped",
				Status:      TargetBuildStopped,
			}, label, false)
		case outcome == TargetBuilt:
			r.add(&RunResult{
				Time:        time.Now(),
				Duration:    time.Since(start),
				Label:       label,
				Description: label + " complete",
				Status:      TargetBuilt,
			}, label, false)
		default:
			log.Fatalf("Invalid build outcome")
		}

	}
}

func (r *RunState) add(result *RunResult, previous string, active bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if s, ok := r.state[result.Label]; ok {
		s.Status = result.Status
		s.Err = result.Err
		s.Description = result.Description
		s.Duration = result.Duration
	} else {
		r.state[result.Label] = &BuildTargetState{
			StartAt:     result.Time,
			Label:       result.Label,
			Status:      result.Status,
			Err:         result.Err,
			Description: result.Description,
			Duration:    result.Duration,
		}
		r.Ordered = append(r.Ordered, result.Label)
	}
	switch {
	case result.Status == TargetBuildFailed:
		r.Failure++
		r.Attempted++
		if r.runOptions.bail && !r.runOptions.stream {
			r.done <- result.Label
		}
	case result.Status == TargetCached:
		r.Cached++
		r.Attempted++
	case result.Status == TargetBuilt:
		r.Success++
		r.Attempted++
	}
}

func (r *RunState) Listen(Ui cli.Ui, startAt time.Time) {
	if r.runOptions.stream {
		return
	}
	r.ticker = time.NewTicker(100 * time.Millisecond)
	r.done = make(chan string)
	lineBuffer := 10
	go func(r *RunState, Ui cli.Ui) {
		z := r
		i := 0
		for {
			select {
			case outcome := <-z.done:
				if !r.runOptions.stream {
					if outcome == "done" {
						if i != 0 {
							cursor.EraseLinesAbove(os.Stdout, lineBuffer+2)
						}
					} else {
						if i != 0 {
							cursor.EraseLinesAbove(os.Stdout, lineBuffer+2)
						}
						z.Render(Ui, startAt, i, lineBuffer)
					}
				}
			case <-z.ticker.C:
				if !r.runOptions.stream {
					if i != 0 {
						cursor.EraseLinesAbove(os.Stdout, lineBuffer+2)
					}
					z.Render(Ui, startAt, i, lineBuffer)
					i++
				}
			default:
				continue
			}
		}
	}(r, Ui)

}

func (r *RunState) Render(ui cli.Ui, startAt time.Time, renderCount int, lineBuffer int) {
	r.mu.Lock()
	defer r.mu.Unlock()
	idx := 0
	buf := len(r.Ordered)
	if buf > lineBuffer {
		idx = buf - lineBuffer
	}
	tStr := fmt.Sprintf("%.2fs", time.Since(startAt).Seconds())
	ui.Output(util.Sprintf("${BOLD}>>> TURBO${RESET}"))
	ui.Output(util.Sprintf("${BOLD}>>> BUILDING%s(%s)${RESET}", strings.Repeat(".", 52-len(tStr)), tStr))

	// In order to simplify the output, we want to fill in n < 10 with IDLE
	// TODO: we might want to match this up with --concurrency flag
	fillOrder := r.Ordered[idx:]
	if len(r.Ordered[idx:]) <= lineBuffer {
		for i := 0; i < lineBuffer-len(r.Ordered); i++ {
			fillOrder = append(fillOrder, "IDLE")
		}
	}
	for _, k := range fillOrder {
		if item, ok := r.state[k]; ok {
			t := fmt.Sprintf("%.2fs", time.Since(item.StartAt).Seconds())
			d := fmt.Sprintf("%.2fs", item.Duration.Seconds())
			fill := 60 - len(item.Label)
			switch r.state[k].Status {
			case TargetBuilding:
				ui.Output(util.Sprintf("${WHITE}%s %s%s(%s)${RESET}", " • ", k, strings.Repeat(".", fill-len(t)), t))
			case TargetCached:
				d = item.Duration.Truncate(time.Millisecond * 100).String()
				ui.Output(util.Sprintf("${GREY}%s %s%s(%s)${RESET}", " ✓ ", k, strings.Repeat(".", fill-len(d)), d))
			case TargetBuilt:
				ui.Output(util.Sprintf("${GREEN}%s %s%s(%s)${RESET}", " ✓ ", k, strings.Repeat(".", fill-len(d)), d))
			case TargetBuildFailed:
				ui.Output(util.Sprintf("${RED}%s %s%s(%s)${RESET}", " ˣ ", k, strings.Repeat(".", fill-len(d)), d))
			default:
				ui.Output(util.Sprintf("${GREY}%s %s%s(%s)${RESET}", " ✓ ", k, strings.Repeat(".", fill-len(d)), d))
			}
		} else {
			ui.Output(util.Sprintf("${GREY}%s %s%s${RESET}", " - ", k, strings.Repeat(".", 62-len(k))))
		}
	}
}

func (r *RunState) Close(Ui cli.Ui, filename string) error {
	outputPath := chrometracing.Path()
	name := fmt.Sprintf("turbo-%s.trace", time.Now().Format(time.RFC3339))
	if filename != "" {
		name = filename
	}
	if outputPath != "" {
		if err := fs.CopyFile(chrometracing.Path(), name, fs.DirPermissions); err != nil {
			return err
		}
	}

	if !r.runOptions.stream {
		r.ticker.Stop()
		r.done <- "done"
	}
	maybeFullTurbo := ""
	if r.Cached == r.Attempted {
		maybeFullTurbo = ui.Rainbow(">>> FULL TURBO")
	}
	if r.runOptions.stream {
		Ui.Output("") // Clear the line
		Ui.Output(util.Sprintf("${BOLD} Tasks:${BOLD_GREEN}    %v successful${RESET}${GRAY}, %v total", r.Cached+r.Success, r.Attempted))
		Ui.Output(util.Sprintf("${BOLD}Cached:    %v cached${RESET}${GRAY}, %v total", r.Cached, r.Attempted))
		Ui.Output(util.Sprintf("${BOLD}  Time:    %v${RESET} %v", time.Since(r.startedAt).Truncate(time.Millisecond), maybeFullTurbo))
		Ui.Output("")
	} else {

		incrementality := fmt.Sprintf("%.f%% incremental", math.Round(float64(r.Cached)/float64(r.Attempted)*100))
		if r.Failure > 0 {
			r.Render(Ui, r.startedAt, 3, len(r.Ordered))
			Ui.Output(util.Sprintf("${BOLD_RED}>>> BUILDING...FINISHED WITH ERRORS${RESET} ${GREY}(%s) %s${RESET} %s${RESET}", time.Since(r.startedAt).Truncate(time.Millisecond).String(), incrementality, maybeFullTurbo))
		} else {
			Ui.Output(util.Sprintf("${BOLD}>>> TURBO${RESET}"))
			Ui.Output(util.Sprintf("${BOLD}>>> BUILDING...FINISHED${RESET} ${GREY}(%s) %s${RESET} %s${RESET}", time.Since(r.startedAt).Truncate(time.Millisecond).String(), incrementality, maybeFullTurbo))
		}

	}
	return nil
}
