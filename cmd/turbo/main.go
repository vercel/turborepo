package main

import (
	"fmt"
	"os"
	"runtime/debug"
	"strings"
	"time"
	"turbo/internal/config"
	"turbo/internal/info"
	"turbo/internal/login"
	prune "turbo/internal/prune"
	"turbo/internal/run"
	uiPkg "turbo/internal/ui"
	"turbo/internal/util"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

func main() {
	args := os.Args[1:]
	heapFile := ""
	traceFile := ""
	cpuprofileFile := ""
	argsEnd := 0
	for _, arg := range args {
		switch {
		case strings.HasPrefix(arg, "--heap="):
			heapFile = arg[len("--heap="):]

		case strings.HasPrefix(arg, "--trace="):
			traceFile = arg[len("--trace="):]

		case strings.HasPrefix(arg, "--cpuprofile="):
			cpuprofileFile = arg[len("--cpuprofile="):]
		default:
			// Strip any arguments that were handled above
			args[argsEnd] = arg
			argsEnd++
		}
	}
	args = args[:argsEnd]
	c := cli.NewCLI("turbo", turboVersion)

	util.InitPrintf()
	ui := &cli.ColoredUi{
		Ui: &cli.BasicUi{
			Reader:      os.Stdin,
			Writer:      os.Stdout,
			ErrorWriter: os.Stderr,
		},
		OutputColor: cli.UiColorNone,
		InfoColor:   cli.UiColorNone,
		WarnColor:   cli.UiColorYellow,
		ErrorColor:  cli.UiColorRed,
	}

	c.Args = args
	c.HelpWriter = os.Stdout
	c.ErrorWriter = os.Stderr
	// Parse and validate cmd line flags and env vars
	cf, err := config.ParseAndValidate(c.Args, ui)
	if err != nil {
		ui.Error(fmt.Sprintf("%s %s", uiPkg.ERROR_PREFIX, color.RedString(err.Error())))
		os.Exit(1)
	}
	c.HiddenCommands = []string{"graph"}
	c.Commands = map[string]cli.CommandFactory{
		"run": func() (cli.Command, error) {
			return &run.RunCommand{Config: cf, Ui: ui},
				nil
		},
		"prune": func() (cli.Command, error) {
			return &prune.PruneCommand{Config: cf, Ui: ui}, nil
		},
		"graph": func() (cli.Command, error) {
			return &info.GraphCommand{Config: cf, Ui: ui}, nil
		},
		"login": func() (cli.Command, error) {
			return &login.LoginCommand{Config: cf, Ui: ui}, nil
		},
		"logout": func() (cli.Command, error) {
			return &login.LogoutCommand{Config: cf, Ui: ui}, nil
		},
		"me": func() (cli.Command, error) {
			return &login.MeCommand{Config: cf, Ui: ui}, nil
		},
		"link": func() (cli.Command, error) {
			return &login.LinkCommand{Config: cf, Ui: ui}, nil
		},
	}

	// Capture the defer statements below so the "done" message comes last
	exitCode := 1
	func() {
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
			// Don't disable the GC if this is a long-running process
			isServe := false
			for _, arg := range args {
				if arg == "--no-cache" {
					isServe = true
					break
				}
			}

			// Disable the GC since we're just going to allocate a bunch of memory
			// and then exit anyway. This speedup is not insignificant. Make sure to
			// only do this here once we know that we're not going to be a long-lived
			// process though.
			if !isServe {
				debug.SetGCPercent(-1)
			}

			exitCode, err = c.Run()
			if err != nil {
				ui.Error(err.Error())
			}
		}
	}()
	os.Exit(exitCode)
}
