package runcache

import (
	"bytes"
	"io"
)

type prefixedWriter struct {
	underlyingWriter io.Writer
	prefix           string
}

func (writer prefixedWriter) Write(payload []byte) (n int, err error) {
	buf := bytes.NewBuffer([]byte{})
	newLine := true
	for _, data := range payload {
		if newLine {
			buf.WriteString(writer.prefix)
			newLine = false
		}

		buf.WriteByte(data)

		if data == '\n' {
			newLine = true
		}
	}

	return writer.underlyingWriter.Write(buf.Bytes())
}
