package variants

import (
	"fmt"
	"io"
	"os"

	"github.com/vercel/turborepo/cli/internal/util"

	"github.com/fatih/color"
)

var successPrefix = color.New(color.Bold, color.FgGreen, color.ReverseVideo).Sprint(" SUCCESS ")
var warningPrefix = color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")
var errorPrefix = color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")

type Default struct {
	Writer      io.Writer
	ErrorWriter io.Writer
}

func NewDefault() *Default {
	return &Default{
		Writer:      os.Stdout,
		ErrorWriter: os.Stderr,
	}
}

func (u *Default) output(msg string) {
	fmt.Fprintln(u.Writer, msg)
}

func (u *Default) Error(err error) {
	fmt.Fprintln(u.ErrorWriter, err.Error())
}

func (u *Default) Printf(format string, args ...interface{}) {
	u.output(util.Sprintf(format, args...))
}

func (u *Default) Successf(format string, args ...interface{}) string {
	msg := fmt.Sprintf(format, args...)
	return fmt.Sprintf("%s%s", successPrefix, color.GreenString(" %v", msg))
}

func (u *Default) Warnf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s", warningPrefix, color.YellowString(" %v", err))
}

func (u *Default) Errorf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s", errorPrefix, color.RedString(" %v", err))
}
