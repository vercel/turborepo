// Package cmd holds the root cobra command for turbo
package cmd

import (
	"context"
	"fmt"
	"os"
	"runtime/pprof"
	"runtime/trace"

	"github.com/pkg/errors"
	"github.com/spf13/cobra"
	"github.com/spf13/pflag"
	"github.com/vercel/turbo/cli/internal/cmd/auth"
	"github.com/vercel/turbo/cli/internal/cmd/info"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/daemon"
	"github.com/vercel/turbo/cli/internal/login"
	"github.com/vercel/turbo/cli/internal/process"
	"github.com/vercel/turbo/cli/internal/prune"
	"github.com/vercel/turbo/cli/internal/run"
	"github.com/vercel/turbo/cli/internal/signals"
	"github.com/vercel/turbo/cli/internal/turbostate"
	"github.com/vercel/turbo/cli/internal/util"
)

type execOpts struct {
	heapFile       string
	cpuProfileFile string
	traceFile      string
}

func (eo *execOpts) addFlags(flags *pflag.FlagSet) {
	// Note that these are relative to the actual CWD, and do not respect the --cwd flag.
	// This is because a user likely wants to inspect them after execution, and may not immediately
	// know the repo root, depending on how turbo was invoked.
	flags.StringVar(&eo.heapFile, "heap", "", "Specify a file to save a pprof heap profile")
	flags.StringVar(&eo.cpuProfileFile, "cpuprofile", "", "Specify a file to save a cpu profile")
	flags.StringVar(&eo.traceFile, "trace", "", "Specify a file to save a pprof trace")
}

// RunWithArgs runs turbo with the specified arguments. The arguments should not
// include the binary being invoked (e.g. "turbo").
func RunWithArgs(args []string, turboVersion string) int {
	util.InitPrintf()
	// TODO: replace this with a context
	signalWatcher := signals.NewWatcher()
	helper := cmdutil.NewHelper(turboVersion)
	root := getCmd(helper, signalWatcher)
	resolvedArgs := resolveArgs(root, args)
	defer helper.Cleanup(root.Flags())
	root.SetArgs(resolvedArgs)

	doneCh := make(chan struct{})
	var execErr error
	go func() {
		execErr = root.Execute()
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
			return 1
		}
		return 0
	case <-signalWatcher.Done():
		// We caught a signal, which already called the close handlers
		return 1
	}
}

// RunWithTurboState runs turbo with the TurboState that is passed from the Rust side.
func RunWithTurboState(state turbostate.TurboState, turboVersion string) int {
	util.InitPrintf()
	// TODO: replace this with a context
	signalWatcher := signals.NewWatcher()
	helper := cmdutil.NewHelper(turboVersion)
	ctx := context.Background()

	if state.ParsedArgs.Trace != nil {
		cleanup, err := createTraceFile(*state.ParsedArgs.Trace)
		if err != nil {
			fmt.Printf("Failed to create trace file: %v\n", err)
			return 1
		}
		helper.RegisterCleanup(cleanup)
	}
	if state.ParsedArgs.Heap != nil {
		cleanup, err := createHeapFile(*state.ParsedArgs.Heap)
		if err != nil {
			fmt.Printf("Failed to create heap file: %v\n", err)
			return 1
		}
		helper.RegisterCleanup(cleanup)
	}
	if state.ParsedArgs.CPUProfile != nil {
		cleanup, err := createCpuprofileFile(*state.ParsedArgs.CPUProfile)
		if err != nil {
			fmt.Printf("Failed to create CPU profile file: %v\n", err)
			return 1
		}
		helper.RegisterCleanup(cleanup)
	}

	defer helper.CleanupWithArgs(&state.ParsedArgs)

	doneCh := make(chan struct{})
	var execErr error
	go func() {
		command := state.ParsedArgs.Command
		if command.Daemon != nil {
			execErr = daemon.Run(ctx, helper, &state.ParsedArgs, signalWatcher)
		} else if command.Link != nil {
			execErr = login.RunLink(helper, &state.ParsedArgs)
		} else if command.Login != nil {
			execErr = login.RunLogin(helper, &state.ParsedArgs, ctx)
		} else if command.Logout != nil {
			execErr = auth.RunLogout(helper, &state.ParsedArgs)
		} else if command.Prune != nil {
			execErr = prune.Run(helper, &state.ParsedArgs)
		} else if command.Unlink != nil {
			execErr = auth.RunUnlink(helper, &state.ParsedArgs)
		} else if command.Run != nil {
			execErr = fmt.Errorf("Command not handled %v\n", command)
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
			return 1
		}
		return 0
	case <-signalWatcher.Done():
		// We caught a signal, which already called the close handlers
		return 1
	}
}

const _defaultCmd string = "run"

// resolveArgs adds a default command to the supplied arguments if none exists.
func resolveArgs(root *cobra.Command, args []string) []string {
	for _, arg := range args {
		if arg == "--help" || arg == "-h" || arg == "--version" || arg == "completion" {
			return args
		}
	}
	cmd, _, err := root.Traverse(args)
	if err != nil {
		// The command is going to error, but defer to cobra
		// to handle it
		return args
	} else if cmd.Name() == root.Name() {
		// We resolved to the root, and this is not help or version,
		// so prepend our default command
		return append([]string{_defaultCmd}, args...)
	}
	// We resolved to something other than the root command, no need for a default
	return args
}

// getCmd returns the root cobra command
func getCmd(helper *cmdutil.Helper, signalWatcher *signals.Watcher) *cobra.Command {
	execOpts := &execOpts{}

	cmd := &cobra.Command{
		Use:              "turbo",
		Short:            "The build system that makes ship happen",
		TraverseChildren: true,
		Version:          helper.TurboVersion,
		PersistentPreRunE: func(cmd *cobra.Command, args []string) error {
			if execOpts.traceFile != "" {
				cleanup, err := createTraceFile(execOpts.traceFile)
				if err != nil {
					return err
				}
				helper.RegisterCleanup(cleanup)
			}
			if execOpts.heapFile != "" {
				cleanup, err := createHeapFile(execOpts.heapFile)
				if err != nil {
					return err
				}
				helper.RegisterCleanup(cleanup)
			}
			if execOpts.cpuProfileFile != "" {
				cleanup, err := createCpuprofileFile(execOpts.cpuProfileFile)
				if err != nil {
					return err
				}
				helper.RegisterCleanup(cleanup)
			}
			return nil
		},
	}
	cmd.SetVersionTemplate("{{.Version}}\n")
	flags := cmd.PersistentFlags()
	helper.AddFlags(flags)
	execOpts.addFlags(flags)
	cmd.AddCommand(login.NewLinkCommand(helper))
	cmd.AddCommand(login.NewLoginCommand(helper))
	cmd.AddCommand(auth.LogoutCmd(helper))
	cmd.AddCommand(auth.UnlinkCmd(helper))
	cmd.AddCommand(info.BinCmd(helper))
	cmd.AddCommand(daemon.GetCmd(helper, signalWatcher))
	cmd.AddCommand(prune.GetCmd(helper))
	cmd.AddCommand(run.GetCmd(helper, signalWatcher))
	return cmd
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
