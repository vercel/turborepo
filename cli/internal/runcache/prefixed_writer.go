package runcache

import (
	"io"
)

// prefixedWriter is responsible to write the log with the `underlyingWriter`.
// It updates the log to write by adding the `prefix` string to each new line.
type prefixedWriter struct {
	underlyingWriter io.Writer
	prefix           string
}

// Writes the given `payload` and add the `prefix` to each new line.
func (writer prefixedWriter) Write(payload []byte) (n int, err error) {
	newLine := true
	for _, data := range payload {
		if newLine {
			if n, err = writer.underlyingWriter.Write([]byte(writer.prefix)); err != nil {
				return n, err
			}
			newLine = false
		}

		if n, err = writer.underlyingWriter.Write([]byte{data}); err != nil {
			return n, err
		}

		if data == '\n' {
			newLine = true
		}
	}

	return n, err
}
