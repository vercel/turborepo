// Package cmd holds the root cobra command for turbo
package cmd

import (
	"context"
	"fmt"
	"os"
	"runtime/pprof"
	"runtime/trace"

	"github.com/pkg/errors"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/daemon"
	"github.com/vercel/turbo/cli/internal/process"
	"github.com/vercel/turbo/cli/internal/prune"
	"github.com/vercel/turbo/cli/internal/run"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
)

func initializeOutputFiles(helper *cmdutil.Helper, parsedArgs *turbostate.ParsedArgsFromRust) error {
	if parsedArgs.Trace != "" {
		cleanup, err := createTraceFile(parsedArgs.Trace)
		if err != nil {
			return fmt.Errorf("failed to create trace file: %v", err)
		}
		helper.RegisterCleanup(cleanup)
	}
	if parsedArgs.Heap != "" {
		cleanup, err := createHeapFile(parsedArgs.Heap)
		if err != nil {
			return fmt.Errorf("failed to create heap file: %v", err)
		}
		helper.RegisterCleanup(cleanup)
	}
	if parsedArgs.CPUProfile != "" {
		cleanup, err := createCpuprofileFile(parsedArgs.CPUProfile)
		if err != nil {
			return fmt.Errorf("failed to create CPU profile file: %v", err)
		}
		helper.RegisterCleanup(cleanup)
	}

	return nil
}

// RunWithArgs runs turbo with the ParsedArgsFromRust that is passed from the Rust side.
func RunWithArgs(args *turbostate.ParsedArgsFromRust, turboVersion string) int {
	util.InitPrintf()
	// TODO: replace this with a context
	signalWatcher := signals.NewWatcher()
	helper := cmdutil.NewHelper(turboVersion, args)
	ctx := context.Background()

	err := initializeOutputFiles(helper, args)
	if err != nil {
		fmt.Printf("%v", err)
		return 1
	}
	defer helper.Cleanup(args)

	doneCh := make(chan struct{})
	var execErr error
	go func() {
		command := args.Command
		if command.Daemon != nil {
			execErr = daemon.ExecuteDaemon(ctx, helper, signalWatcher, args)
		} else if command.Prune != nil {
			execErr = prune.ExecutePrune(helper, args)
		} else if command.Run != nil {
			execErr = run.ExecuteRun(ctx, helper, signalWatcher, args)
		} else {
			execErr = fmt.Errorf("unknown command: %v", command)
		}

		close(doneCh)
	}()

	// Wait for either our command to finish, in which case we need to clean up,
	// or to receive a signal, in which case the signal handler above does the cleanup
	select {
	case <-doneCh:
		// We finished whatever task we were running
		signalWatcher.Close()
		exitErr := &process.ChildExit{}
		if errors.As(execErr, &exitErr) {
			return exitErr.ExitCode
		} else if execErr != nil {
			fmt.Printf("Turbo error: %v\n", execErr)
			return 1
		}
		return 0
	case <-signalWatcher.Done():
		// We caught a signal, which already called the close handlers
		return 1
	}
}

type profileCleanup func() error

// Close implements io.Close for profileCleanup
func (pc profileCleanup) Close() error {
	return pc()
}

// To view a CPU trace, use "go tool trace [file]". Note that the trace
// viewer doesn't work under Windows Subsystem for Linux for some reason.
func createTraceFile(traceFile string) (profileCleanup, error) {
	f, err := os.Create(traceFile)
	if err != nil {
		return nil, errors.Wrapf(err, "failed to create trace file: %v", traceFile)
	}
	if err := trace.Start(f); err != nil {
		return nil, errors.Wrap(err, "failed to start tracing")
	}
	return func() error {
		trace.Stop()
		return f.Close()
	}, nil
}

// To view a heap trace, use "go tool pprof [file]" and type "top". You can
// also drop it into https://speedscope.app and use the "left heavy" or
// "sandwich" view modes.
func createHeapFile(heapFile string) (profileCleanup, error) {
	f, err := os.Create(heapFile)
	if err != nil {
		return nil, errors.Wrapf(err, "failed to create heap file: %v", heapFile)
	}
	return func() error {
		if err := pprof.WriteHeapProfile(f); err != nil {
			// we don't care if we fail to close the file we just failed to write to
			_ = f.Close()
			return errors.Wrapf(err, "failed to write heap file: %v", heapFile)
		}
		return f.Close()
	}, nil
}

// To view a CPU profile, drop the file into https://speedscope.app.
// Note: Running the CPU profiler doesn't work under Windows subsystem for
// Linux. The profiler has to be built for native Windows and run using the
// command prompt instead.
func createCpuprofileFile(cpuprofileFile string) (profileCleanup, error) {
	f, err := os.Create(cpuprofileFile)
	if err != nil {
		return nil, errors.Wrapf(err, "failed to create cpuprofile file: %v", cpuprofileFile)
	}
	if err := pprof.StartCPUProfile(f); err != nil {
		return nil, errors.Wrap(err, "failed to start CPU profiling")
	}
	return func() error {
		pprof.StopCPUProfile()
		return f.Close()
	}, nil
}
