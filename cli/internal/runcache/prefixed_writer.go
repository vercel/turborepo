package runcache

import (
	"io"
)

type prefixedWriter struct {
	underlyingWriter io.Writer
	prefix           string
}

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
