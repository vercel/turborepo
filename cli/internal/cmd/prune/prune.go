package prune

import (
	"os"

	"github.com/spf13/cobra"
	"github.com/vercel/turborepo/cli/internal/cmdutil"
)

func PruneCmd(ch *cmdutil.Helper) *cobra.Command {
	var opts struct {
		scope  string
		docker bool
		cwd    string
	}

	cmd := &cobra.Command{
		Use:   "prune",
		Short: "Prepare a subset of your monorepo",
		RunE: func(cmd *cobra.Command, args []string) error {
			return nil
		},
	}

	path, err := os.Getwd()
	if err != nil {
		return nil
	}

	cmd.Flags().StringVar(&opts.scope, "scope", "", "package to act as entry point for pruned monorepo")
	cmd.Flags().BoolVarP(&opts.docker, "docker", "d", false, "output pruned workspace into 'full' and 'json' directories optimized for Docker layer caching")
	cmd.Flags().StringVar(&opts.cwd, "cwd", path, "directory to execute command in")

	return cmd
}
