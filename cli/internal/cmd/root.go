package cmd

import (
	"context"
	"errors"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmd/info"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/logger"
)

var rootCmd = &cobra.Command{
	Use:     "turbo <command> [<args>]",
	Short:   "Turborepo is a very fast Javascript build tool",
	Long: `The High-performance Build System for JavaScript & TypeScript Codebases.
Complete documentation is available at https://turborepo.com.`,
}

func Execute(ctx context.Context, version string) int {
	var debug bool

	err := runCmd(ctx, version, &debug)
	if err == nil {
		return 0
	}

	logger := logger.NewLogger()
	logger.Printf(err)

	var cmdErr *cmdutil.Error
	if errors.As(err, &cmdErr) {
		return cmdErr.ExitCode
	}

	return 1
}

func runCmd(ctx context.Context, version string, debug *bool) error {
	rootCmd.SilenceUsage = true
	rootCmd.SilenceErrors = true

	rootCmd.Version = version
	rootCmd.SetVersionTemplate(`{{printf "%s" .Version}}
`)

	rootCmd.PersistentFlags().BoolVarP(debug, "debug", "d", false, "enable debug mode")

	ch := &cmdutil.Helper{
		Logger: logger.NewLogger(),
	}
	ch.SetDebug(debug)

	rootCmd.AddCommand(info.BinCmd(ch))

	return rootCmd.ExecuteContext(ctx)
}
