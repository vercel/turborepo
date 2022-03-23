package main

import (
	"os"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/cmd"
	"github.com/vercel/turborepo/cli/internal/process"
)

func main() {
	exitCode := 1
	doneCh := make(chan struct{})
	processes := process.NewManager(hclog.Default().Named("processes"))
	signalCh := watchSignals(func() { processes.Close() })

	func() {
		defer func() { close(doneCh) }()
		exitCode = cmd.Execute(turboVersion, processes)
	}()

	// Wait for either our command to finish, in which case we need to clean up,
	// or to receive a signal, in which case the signal handler above does the cleanup
	select {
	case <-doneCh:
		processes.Close()
	case <-signalCh:
	}

	os.Exit(exitCode)
}
