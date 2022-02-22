package cmd

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

// binCmd represents the bin command
var binCmd = &cobra.Command{
	Use:   "bin",
	Short: "Get the path to the Turbo binary",
	Run: func(cmd *cobra.Command, args []string) {
		path, err := os.Executable()
		if err != nil {
			cobra.CheckErr(fmt.Errorf("could not get path to turbo binary: %w", err))
		}
		
		fmt.Println(path)
	},
}

func init() {
	rootCmd.AddCommand(binCmd)
}
