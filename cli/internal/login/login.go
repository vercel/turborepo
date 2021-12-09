package login

import (
	"bytes"
	"fmt"
	"log"
	"os/exec"
	"path/filepath"
	"strings"
	"turbo/internal/config"

	"github.com/mitchellh/cli"
)

// LoginCommand is a Command implementation that tells Turbo to run a task
type LoginCommand struct {
	Config *config.Config
	Ui     *cli.ColoredUi
}

// Synopsis of run command
func (c *LoginCommand) Synopsis() string {
	return "Login to your Vercel account"
}

// Help returns information about the `run` command
func (c *LoginCommand) Help() string {
	helpText := `
Usage: turbo login

  Login to your Vercel account
`
	return strings.TrimSpace(helpText)
}

// Run executes tasks in the monorepo
func (c *LoginCommand) Run(args []string) int {
	cmd := exec.Command("node", filepath.FromSlash("login.js"))
	var outb, errb bytes.Buffer
	cmd.Stdout = &outb
	cmd.Stderr = &errb
	err := cmd.Run()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("out:", outb.String(), "err:", errb.String())
	return 0
}
