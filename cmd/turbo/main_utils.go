package main

import (
	"fmt"
	"log"
	"os"
	"runtime/pprof"
	"runtime/trace"

	"github.com/fatih/color"
)

var logger = log.New(os.Stderr, "", 0)

func printErrorToStderr(args []string, errString string) {
	logger.Printf("args: %v", args)
	logger.Print(color.RedString("[ERROR] %v", errString))
}

func createTraceFile(osArgs []string, traceFile string) func() {
	f, err := os.Create(traceFile)
	if err != nil {
		printErrorToStderr(osArgs, fmt.Sprintf(
			"Failed to create trace file: %s", err.Error()))
		return nil
	}
	trace.Start(f)
	return func() {
		trace.Stop()
		f.Close()
	}
}

func createHeapFile(osArgs []string, heapFile string) func() {
	f, err := os.Create(heapFile)
	if err != nil {
		printErrorToStderr(osArgs, fmt.Sprintf(
			"Failed to create heap file: %s", err.Error()))
		return nil
	}
	return func() {
		if err := pprof.WriteHeapProfile(f); err != nil {
			printErrorToStderr(osArgs, fmt.Sprintf(
				"Failed to write heap profile: %s", err.Error()))
		}
		f.Close()
	}
}

func createCpuprofileFile(osArgs []string, cpuprofileFile string) func() {
	f, err := os.Create(cpuprofileFile)
	if err != nil {
		printErrorToStderr(osArgs, fmt.Sprintf(
			"Failed to create cpuprofile file: %s", err.Error()))
		return nil
	}
	pprof.StartCPUProfile(f)
	return func() {
		pprof.StopCPUProfile()
		f.Close()
	}
}
