package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/vercel/turbo/cli/internal/cmd"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Printf("go-turbo is expected to be invoked via turbo")
		os.Exit(1)
	}

	executionStateString := os.Args[1]
	var executionState turbostate.ExecutionState
	decoder := json.NewDecoder(strings.NewReader(executionStateString))
	decoder.DisallowUnknownFields()

	err := decoder.Decode(&executionState)
	if err != nil {
		fmt.Printf("Error unmarshalling execution state: %v\n Execution state string: %v\n", err, executionStateString)
		os.Exit(1)
	}

	exitCode := cmd.RunWithExecutionState(&executionState, turboVersion)
	os.Exit(exitCode)
}
