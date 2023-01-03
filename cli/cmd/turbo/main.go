package main

import "C"
import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"

	"github.com/vercel/turbo/cli/internal/cmd"
	"github.com/vercel/turbo/cli/internal/turbostate"
)

func main() {
	reader := bufio.NewReader(os.Stdin)
	argsString, _ := reader.ReadString('\n')

	var args turbostate.ParsedArgsFromRust
	err := json.Unmarshal([]byte(argsString), &args)
	if err != nil {
		fmt.Printf("Error unmarshalling CLI args: %v\n Arg string: %v\n", err, argsString)
		os.Exit(1)
	}

	exitCode := cmd.RunWithArgs(args, turboVersion)
	os.Exit(exitCode)
}

//export nativeRunWithArgs
func nativeRunWithArgs(argsString string) C.uint {
	var args turbostate.ParsedArgsFromRust
	err := json.Unmarshal([]byte(argsString), &args)
	if err != nil {
		fmt.Printf("Error unmarshalling CLI args: %v\n Arg string: %v\n", err, argsString)
		return 1
	}
	exitCode := cmd.RunWithArgs(args, turboVersion)
	return C.uint(exitCode)
}
