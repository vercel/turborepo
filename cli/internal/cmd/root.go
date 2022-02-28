package cmd

import (
	"context"
	"errors"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmd/auth"
	"github.com/vercel/turborepo/cli/internal/cmd/info"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/logger"
)

var rootCmd = &cobra.Command{
	Use:   "turbo <command> [<args>]",
	Short: "Turborepo is a very fast Javascript build tool",
	Long: `The High-performance Build System for JavaScript & TypeScript Codebases.
Complete documentation is available at https://turborepo.com.`,
}

func Execute(ctx context.Context, version string) int {
	logger := logger.NewLogger()

	err := runCmd(ctx, logger, version)
	if err == nil {
		return 0
	}

	logger.Printf(err)

	var cmdErr *cmdutil.Error
	if errors.As(err, &cmdErr) {
		return cmdErr.ExitCode
	}

	return 1
}

func runCmd(ctx context.Context, logger *logger.Logger, version string) error {
	rootCmd.SilenceUsage = true
	rootCmd.SilenceErrors = true
	rootCmd.CompletionOptions.DisableDefaultCmd = true

	rootCmd.Version = version
	rootCmd.SetVersionTemplate(`{{printf "%s" .Version}}
`)

	cfg, err := config.New(logger, version)
	if err != nil {
		return err
	}

	rootCmd.PersistentFlags().CountVarP(&cfg.Level, "level", "l", "set log level")
	rootCmd.PersistentFlags().BoolVar(&cfg.NoColor, "no-color", false, "disable color output")
	rootCmd.PersistentFlags().StringVar(&cfg.Token, "token", cfg.Token, "vercel token")
	rootCmd.PersistentFlags().StringVar(&cfg.TeamSlug, "team", cfg.TeamSlug, "vercel team slug")
	rootCmd.PersistentFlags().StringVar(&cfg.ApiUrl, "api", cfg.ApiUrl, "vercel api url")
	rootCmd.PersistentFlags().StringVar(&cfg.LoginUrl, "url", cfg.LoginUrl, "vercel login url")

	rootCmd.PersistentFlags().Lookup("token").DefValue = ""

	ch := &cmdutil.Helper{
		Logger: logger,
		Config: cfg,
	}

	rootCmd.PersistentPreRunE = ch.PreRun()

	rootCmd.AddCommand(info.BinCmd(ch))
	rootCmd.AddCommand(auth.LinkCmd(ch))
	rootCmd.AddCommand(auth.UnlinkCmd(ch))
	rootCmd.AddCommand(auth.LoginCmd(ch))
	rootCmd.AddCommand(auth.LogoutCmd(ch))

	return rootCmd.ExecuteContext(ctx)
}
