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
	var totalLength int
	for _, data := range payload {
		if newLine {
			_, err = writer.underlyingWriter.Write([]byte(writer.prefix))
			if err != nil {
				return totalLength, err
			}
			newLine = false
		}

		n, err = writer.underlyingWriter.Write([]byte{data})
		totalLength += n
		if err != nil {
			return totalLength, err
		}

		if data == '\n' {
			newLine = true
		}
	}

	return totalLength, err
}
