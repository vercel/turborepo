package logger

import (
	"fmt"
	"io"
	"os"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
)

var IsTTY = isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())

type Logger struct {
	out io.Writer
}

func NewLogger() *Logger {
	return &Logger{
		out: os.Stdout,
	}
}

func (l *Logger) Printf(args ...interface{}) {
	fmt.Fprintln(l.out, args...)
}

func (l *Logger) Sucessf(format string, args ...interface{}) string {
	msg := fmt.Sprintf(format, args...)
	successPrefix := color.New(color.Bold, color.FgGreen, color.ReverseVideo).Sprint(" SUCCESS ")

	return fmt.Sprintf("%s%s", successPrefix, color.GreenString(" %v", msg))
}

func (l *Logger) Warnf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	warnPrefix := color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")

	return fmt.Errorf("%s%s", warnPrefix, color.YellowString(" %v", err))
}

func (l *Logger) Errorf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	errorPrefix := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")

	return fmt.Errorf("%s%s", errorPrefix, color.RedString(" %v", err))
}
