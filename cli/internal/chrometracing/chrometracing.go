// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Package chrometracing writes per-process Chrome trace_event files that can be
// loaded into chrome://tracing.
package chrometracing

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/google/chrometracing/traceinternal"
)

var trace = struct {
	start time.Time
	pid   uint64

	fileMu sync.Mutex
	file   *os.File
}{
	pid: uint64(os.Getpid()),
}

var out = setup(false)

// Path returns the full path of the chrome://tracing trace_event file for
// display in log messages.
func Path() string { return out }

// EnableTracing turns on tracing, regardless of running in a test or
// not. Tracing is enabled by default if the CHROMETRACING_DIR environment
// variable is present and non-empty.
func EnableTracing() {
	trace.fileMu.Lock()
	alreadyEnabled := trace.file != nil
	trace.fileMu.Unlock()
	if alreadyEnabled {
		return
	}
	out = setup(true)
}

func setup(overrideEnable bool) string {
	inTest := os.Getenv("TEST_TMPDIR") != ""
	explicitlyEnabled := os.Getenv("CHROMETRACING_DIR") != ""
	enableTracing := inTest || explicitlyEnabled || overrideEnable
	if !enableTracing {
		return ""
	}

	var err error
	dir := os.Getenv("TEST_UNDECLARED_OUTPUTS_DIR")
	if dir == "" {
		dir = os.Getenv("CHROMETRACING_DIR")
	}
	if dir == "" {
		dir = os.TempDir()
	}
	fn := filepath.Join(dir, fmt.Sprintf("%s.%d.trace", filepath.Base(os.Args[0]), trace.pid))
	trace.file, err = os.OpenFile(fn, os.O_WRONLY|os.O_CREATE|os.O_TRUNC|os.O_EXCL, 0644)
	if err != nil {
		// Using the log package from func init results in an error message
		// being printed.
		fmt.Fprintf(os.Stderr, "continuing without tracing: %v\n", err)
		return ""
	}

	// We only ever open a JSON array. Ending the array is optional as per
	// go/trace_event so that not cleanly finished traces can still be read.
	trace.file.Write([]byte{'['})
	trace.start = time.Now()

	writeEvent(&traceinternal.ViewerEvent{
		Name:  "process_name",
		Phase: "M", // Metadata Event
		Pid:   trace.pid,
		Tid:   trace.pid,
		Arg: struct {
			Name string `json:"name"`
		}{
			Name: strings.Join(os.Args, " "),
		},
	})
	return fn
}

func writeEvent(ev *traceinternal.ViewerEvent) {
	b, err := json.Marshal(&ev)
	if err != nil {
		fmt.Fprintf(os.Stderr, "%v\n", err)
		return
	}
	trace.fileMu.Lock()
	defer trace.fileMu.Unlock()
	if _, err = trace.file.Write(b); err != nil {
		fmt.Fprintf(os.Stderr, "%v\n", err)
		return
	}
	if _, err = trace.file.Write([]byte{',', '\n'}); err != nil {
		fmt.Fprintf(os.Stderr, "%v\n", err)
		return
	}
}

const (
	begin = "B"
	end   = "E"
)

// A PendingEvent represents an ongoing unit of work. The begin trace event has
// already been written, and calling Done will write the end trace event.
type PendingEvent struct {
	name string
	tid  uint64
}

// Done writes the end trace event for this unit of work.
func (pe *PendingEvent) Done() {
	if pe == nil || pe.name == "" || trace.file == nil {
		return
	}
	writeEvent(&traceinternal.ViewerEvent{
		Name:  pe.name,
		Phase: end,
		Pid:   trace.pid,
		Tid:   pe.tid,
		Time:  float64(time.Since(trace.start).Microseconds()),
	})
	releaseTid(pe.tid)
}

// Event logs a unit of work. To instrument a Go function, use e.g.:
//
//	func calcPi() {
//	  defer chrometracing.Event("calculate pi").Done()
//	  // …
//	}
//
// For more finely-granular traces, use e.g.:
//
//	for _, cmd := range commands {
//	  ev := chrometracing.Event("initialize " + cmd.Name)
//	  cmd.Init()
//	  ev.Done()
//	}
func Event(name string) *PendingEvent {
	if trace.file == nil {
		return &PendingEvent{}
	}
	tid := tid()
	writeEvent(&traceinternal.ViewerEvent{
		Name:  name,
		Phase: begin,
		Pid:   trace.pid,
		Tid:   tid,
		Time:  float64(time.Since(trace.start).Microseconds()),
	})
	return &PendingEvent{
		name: name,
		tid:  tid,
	}
}

// tids is a chrome://tracing thread id pool. Go does not permit accessing the
// goroutine id, so we need to maintain our own identifier. The chrome://tracing
// file format requires a numeric thread id, so we just increment whenever we
// need a thread id, and reuse the ones no longer in use.
//
// In practice, parallelized sections of the code (many goroutines) end up using
// only as few thread ids as are concurrently in use, and the rest of the events
// mirror the code call stack nicely. See e.g. http://screen/7MPcAcvXQNUE3JZ
var tids struct {
	sync.Mutex

	// We allocate chrome://tracing thread ids based on the index of the
	// corresponding entry in the used slice.
	used []bool

	// next points to the earliest unused tid to consider for the next tid to
	// hand out. This is purely a performance optimization to avoid O(n) slice
	// iteration.
	next int
}

func tid() uint64 {
	tids.Lock()
	defer tids.Unlock()
	// re-use released tids if any
	for t := tids.next; t < len(tids.used); t++ {
		if !tids.used[t] {
			tids.used[t] = true
			tids.next = t + 1
			return uint64(t)
		}
	}
	// allocate a new tid
	t := len(tids.used)
	tids.used = append(tids.used, true)
	tids.next = t + 1
	return uint64(t)
}

func releaseTid(t uint64) {
	tids.Lock()
	defer tids.Unlock()
	tids.used[int(t)] = false
	if tids.next > int(t) {
		tids.next = int(t)
	}
}
