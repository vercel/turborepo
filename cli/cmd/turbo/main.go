package main

import "C"
import (
	"encoding/json"
	"fmt"
	"os"
	"unsafe"

	"github.com/vercel/turborepo/cli/internal/turbostate"

	"github.com/vercel/turborepo/cli/internal/cmd"
)

func main() {
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion))
}

//export nativeRunWithArgs
func nativeRunWithArgs(argc C.int, argv **C.char, turboStateString string) C.uint {
	arglen := int(argc)
	args := make([]string, arglen)
	for i, arg := range unsafe.Slice(argv, arglen) {
		args[i] = C.GoString(arg)
	}
	fmt.Printf("%v\n", turboStateString)
	turboState := turbostate.TurboState{}
	err := json.Unmarshal([]byte(turboStateString), &turboState)
	if err != nil {
		fmt.Printf("Error unmarshalling turboState: %v\n", err)
		return 1
	}
	exitCode := cmd.RunWithTurboState(turboState, turboVersion)
	return C.uint(exitCode)
}
