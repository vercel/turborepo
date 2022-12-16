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
	fmt.Printf("ERROR: Go binary cannot be used on its own. Please build as c-archive and use with Rust crate")
	os.Exit(1)
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
