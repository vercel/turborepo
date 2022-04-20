package variants

import (
	"io"
	"regexp"
)

const ansiEscapeStr = "[\u001B\u009B][[\\]()#;?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[a-zA-Z\\d]*)*)?\u0007)|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PRZcf-ntqry=><~]))"

var ansiRegex = regexp.MustCompile(ansiEscapeStr)

type Ui interface {
	output(msg string)

	// Output to ErrorWriter
	Error(err error)
	// Format and output to Writer
	Printf(format string, args ...interface{})
	// Format success
	Successf(format string, args ...interface{}) string
	// Format warning
	Warnf(format string, args ...interface{}) error
	// Format error
	Errorf(format string, args ...interface{}) error
}

type stripAnsiWriter struct {
	wrappedWriter io.Writer
}

func (into *stripAnsiWriter) Write(p []byte) (int, error) {
	n, err := into.wrappedWriter.Write(ansiRegex.ReplaceAll(p, []byte{}))
	if err != nil {
		// The number of bytes returned here isn't directly related to the input bytes
		// if ansi color codes were being stripped out, but we are counting on Stdout.Write
		// not failing under typical operation as well.
		return n, err
	}

	// Write must return a non-nil error if it returns n < len(p). Consequently, if the
	// wrappedWrite.Write call succeeded we will return len(p) as the number of bytes
	// written.
	return len(p), nil
}
