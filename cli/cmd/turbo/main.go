package main

import "C"
import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/vercel/turbo/cli/internal/cmd"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

func main() {
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion))
}

//export nativeRunWithTurboState
func nativeRunWithTurboState(turboStateString string) C.uint {
	var turboState turbostate.CLIExecutionStateFromRust
	err := json.Unmarshal([]byte(turboStateString), &turboState)
	if err != nil {
		fmt.Printf("Error unmarshalling turboState: %v\n Turbo state string: %v\n", err, turboStateString)
		return 1
	}
	exitCode := cmd.RunWithTurboState(turboState, turboVersion)
	return C.uint(exitCode)
}
