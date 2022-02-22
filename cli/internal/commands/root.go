package cmd

import (
	"os"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{
	Use:     "turbo <command> [<args>]",
	Version: "1.1.1",
	Short:   "Turborepo is a very fast Javascript build tool",
	Long: `The High-performance Build System for JavaScript & TypeScript Codebases.
Complete documentation is available at https://turborepo.com.`,
}

func Execute() {
	err := rootCmd.Execute()
	if err != nil {
		os.Exit(1)
	}
}

func init() {
	rootCmd.SetVersionTemplate(`{{printf "%s" .Version}}
`)
}
