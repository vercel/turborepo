package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/vercel/turborepo/cli/internal/cmd/auth"
	"github.com/vercel/turborepo/cli/internal/cmd/info"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/daemon"
	"github.com/vercel/turborepo/cli/internal/login"
	prune "github.com/vercel/turborepo/cli/internal/prune"
	"github.com/vercel/turborepo/cli/internal/run"
	"github.com/vercel/turborepo/cli/internal/signals"
	"github.com/vercel/turborepo/cli/internal/ui"
	uiPkg "github.com/vercel/turborepo/cli/internal/ui"
	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

func main() {
	args := os.Args[1:]
	heapFile := ""
	traceFile := ""
	cpuprofileFile := ""
	argsEnd := 0
	colorMode := uiPkg.GetColorModeFromEnv()
	for _, arg := range args {
		switch {
		case strings.HasPrefix(arg, "--heap="):
			heapFile = arg[len("--heap="):]
		case strings.HasPrefix(arg, "--trace="):
			traceFile = arg[len("--trace="):]
		case strings.HasPrefix(arg, "--cpuprofile="):
			cpuprofileFile = arg[len("--cpuprofile="):]
		case arg == "--color":
			colorMode = ui.ColorModeForced
		case arg == "--no-color":
			colorMode = ui.ColorModeSuppressed
		default:
			// Strip any arguments that were handled above
			args[argsEnd] = arg
			argsEnd++
		}
	}
	args = args[:argsEnd]

	ui := ui.BuildColoredUi(colorMode)
	c := cli.NewCLI("turbo", turboVersion)

	util.InitPrintf()

	c.Args = args
	c.HelpWriter = os.Stdout
	c.ErrorWriter = os.Stderr
	// Parse and validate cmd line flags and env vars
	// Note that cf can be nil
	cf, err := config.ParseAndValidate(c.Args, ui, turboVersion, config.DefaultUserConfigPath())
	if err != nil {
		ui.Error(fmt.Sprintf("%s %s", uiPkg.ERROR_PREFIX, color.RedString(err.Error())))
		os.Exit(1)
	}

	signalWatcher := signals.NewWatcher()
	c.HiddenCommands = []string{"graph"}
	c.Commands = map[string]cli.CommandFactory{
		"run": func() (cli.Command, error) {
			return &run.RunCommand{Config: cf, UI: ui, SignalWatcher: signalWatcher},
				nil
		},
		"prune": func() (cli.Command, error) {
			return &prune.PruneCommand{Config: cf, Ui: ui}, nil
		},
		"link": func() (cli.Command, error) {
			return &login.LinkCommand{Config: cf, Ui: ui}, nil
		},
		"unlink": func() (cli.Command, error) {
			return &auth.UnlinkCommand{Config: cf, UI: ui}, nil
		},
		"login": func() (cli.Command, error) {
			return &login.LoginCommand{Config: cf, UI: ui}, nil
		},
		"logout": func() (cli.Command, error) {
			return &auth.LogoutCommand{Config: cf, UI: ui}, nil
		},
		"bin": func() (cli.Command, error) {
			return &info.BinCommand{Config: cf, UI: ui}, nil
		},
		"daemon": func() (cli.Command, error) {
			return &daemon.Command{Config: cf, UI: ui, SignalWatcher: signalWatcher}, nil
		},
	}

	// Capture the defer statements below so the "done" message comes last
	exitCode := 1
	doneCh := make(chan struct{})
	func() {
		defer func() { close(doneCh) }()
		// To view a CPU trace, use "go tool trace [file]". Note that the trace
		// viewer doesn't work under Windows Subsystem for Linux for some reason.
		if traceFile != "" {
			if done := createTraceFile(args, traceFile); done == nil {
				return
			} else {
				defer done()
			}
		}

		// To view a heap trace, use "go tool pprof [file]" and type "top". You can
		// also drop it into https://speedscope.app and use the "left heavy" or
		// "sandwich" view modes.
		if heapFile != "" {
			if done := createHeapFile(args, heapFile); done == nil {
				return
			} else {
				defer done()
			}
		}

		// To view a CPU profile, drop the file into https://speedscope.app.
		// Note: Running the CPU profiler doesn't work under Windows subsystem for
		// Linux. The profiler has to be built for native Windows and run using the
		// command prompt instead.
		if cpuprofileFile != "" {
			if done := createCpuprofileFile(args, cpuprofileFile); done == nil {
				return
			} else {
				defer done()
			}
		}

		if cpuprofileFile != "" {
			// The CPU profiler in Go only runs at 100 Hz, which is far too slow to
			// return useful information for esbuild, since it's so fast. Let's keep
			// running for 30 seconds straight, which should give us 3,000 samples.
			seconds := 30.0
			start := time.Now()
			for time.Since(start).Seconds() < seconds {
				exitCode, err = c.Run()
				if err != nil {
					ui.Error(err.Error())
				}
			}
		} else {
			exitCode, err = c.Run()
			if err != nil {
				ui.Error(err.Error())
			}
		}
	}()
	// Wait for either our command to finish, in which case we need to clean up,
	// or to receive a signal, in which case the signal handler above does the cleanup
	select {
	case <-doneCh:
		// We finished whatever task we were running
		signalWatcher.Close()
	case <-signalWatcher.Done():
		// We caught a signal, which already called the close handlers
	}
	os.Exit(exitCode)
}
