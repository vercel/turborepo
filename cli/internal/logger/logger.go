package logger

import (
	"fmt"
	"io"
	"os"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/vercel/turborepo/cli/internal/util"
)

var IsTTY = isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
var IsCI = os.Getenv("CI") == "true" || os.Getenv("BUILD_NUMBER") == "true" || os.Getenv("TEAMCITY_VERSION") != ""

var successPrefix = color.New(color.Bold, color.FgGreen, color.ReverseVideo).Sprint(" SUCCESS ")
var warningPrefix = color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")
var errorPrefix = color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")

type Logger struct {
	Out io.Writer
}

func New() *Logger {
	return &Logger{
		Out: os.Stdout,
	}
}

func (l *Logger) Printf(format string, args ...interface{}) {
	fmt.Fprintln(os.Stdout, util.Sprintf(format, args...))
}

func (l *Logger) Sucessf(format string, args ...interface{}) string {
	msg := fmt.Sprintf(format, args...)
	return fmt.Sprintf("%s%s", successPrefix, color.GreenString(" %v", msg))
}

func (l *Logger) Warnf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s", warningPrefix, color.YellowString(" %v", err))
}

func (l *Logger) Errorf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s", errorPrefix, color.RedString(" %v", err))
}
