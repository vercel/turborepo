package main

import "C"
import (
	"encoding/json"
	"fmt"
	"os"
	"unsafe"

	"github.com/vercel/turbo/cli/internal/cmd"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

func main() {
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion))
}

//export nativeRunWithArgs
func nativeRunWithArgs(argc C.int, argv **C.char) C.uint {
	arglen := int(argc)
	args := make([]string, arglen)
	for i, arg := range unsafe.Slice(argv, arglen) {
		args[i] = C.GoString(arg)
	}

	exitCode := cmd.RunWithArgs(args, turboVersion)
	return C.uint(exitCode)
}

//export nativeRunWithTurboState
func nativeRunWithTurboState(turboStateString string) C.uint {
	turboState := turbostate.CLIExecutionStateFromRust{}
	err := json.Unmarshal([]byte(turboStateString), &turboState)
	if err != nil {
		fmt.Printf("Error unmarshalling turboState: %v\n", err)
		return 1
	}
	exitCode := cmd.RunWithTurboState(turboState, turboVersion)
	return C.uint(exitCode)
}
