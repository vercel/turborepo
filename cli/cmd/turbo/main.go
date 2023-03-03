package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/vercel/turbo/cli/internal/cmd"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Printf("go-turbo is expected to be invoked via turbo")
		os.Exit(1)
	}

	argsString := os.Args[1]
	var args turbostate.ParsedArgsFromRust
	err := json.Unmarshal([]byte(argsString), &args)
	if err != nil {
		fmt.Printf("Error unmarshalling CLI args: %v\n Arg string: %v\n", err, argsString)
		os.Exit(1)
	}

	exitCode := cmd.RunWithArgs(&args, turboVersion)
	os.Exit(exitCode)
}
