package main

import (
	"os"

	"github.com/vercel/turborepo/cli/internal/cmd"
)

func main() {
	os.Exit(cmd.RunWithArgs(os.Args[1:], turboVersion))
}
