package logger

import (
	"fmt"
	"io"
	"os"

	"github.com/fatih/color"
	"github.com/vercel/turborepo/cli/internal/util"
)

type PrefixedLogger struct {
	out io.Writer

	outputPrefix  string
	successPrefix string
	warningPrefix string
	errorPrefix   string
}

func NewPrefixed(outputPrefix, successPrefix, warningPrefix, errorPrefix string) *PrefixedLogger {
	return &PrefixedLogger{
		out: os.Stdout,

		outputPrefix: outputPrefix,
		successPrefix: successPrefix,
		warningPrefix: warningPrefix,
		errorPrefix: errorPrefix,
	}
}

func (l *PrefixedLogger) Printf(format string, args ...interface{}) {
	fmt.Fprintln(l.out, util.Sprintf(format, args...))
}

func (l *PrefixedLogger) Sucessf(format string, args ...interface{}) string {
	msg := fmt.Sprintf(format, args...)
	return fmt.Sprintf("%s%s%s", successPrefix, l.successPrefix, color.GreenString("%v", msg))
}

func (l *PrefixedLogger) Warnf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s%s", warningPrefix, l.warningPrefix, color.YellowString("%v", err))
}

func (l *PrefixedLogger) Errorf(format string, args ...interface{}) error {
	err := fmt.Errorf(format, args...)
	return fmt.Errorf("%s%s%s", errorPrefix, l.errorPrefix, color.RedString("%v", err))
}

func (l *PrefixedLogger) Output(format string, args ...interface{}) {
	msg := fmt.Sprintf(format, args...)
	l.Printf("%s%s", l.outputPrefix, msg)
}
