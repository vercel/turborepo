// Copyright (c) 2013 Kevin van Zonneveld <kevin@vanzonneveld.net>. All rights reserved.
// Source: https://github.com/kvz/logstreamer
// SPDX-License-Identifier: MIT
package logstreamer

import (
	"bytes"
	"io"
	"log"
	"os"
	"strings"
)

type Logstreamer struct {
	Logger *log.Logger
	buf    *bytes.Buffer
	// If prefix == stdout, colors green
	// If prefix == stderr, colors red
	// Else, prefix is taken as-is, and prepended to anything
	// you throw at Write()
	prefix string
	// if true, saves output in memory
	record  bool
	persist string

	// Adds color to stdout & stderr if terminal supports it
	colorOkay  string
	colorFail  string
	colorReset string
}

func NewLogstreamer(logger *log.Logger, prefix string, record bool) *Logstreamer {
	streamer := &Logstreamer{
		Logger:     logger,
		buf:        bytes.NewBuffer([]byte("")),
		prefix:     prefix,
		record:     record,
		persist:    "",
		colorOkay:  "",
		colorFail:  "",
		colorReset: "",
	}

	if strings.HasPrefix(os.Getenv("TERM"), "xterm") {
		streamer.colorOkay = "\x1b[32m"
		streamer.colorFail = "\x1b[31m"
		streamer.colorReset = "\x1b[0m"
	}

	return streamer
}

func (l *Logstreamer) Write(p []byte) (n int, err error) {
	if n, err = l.buf.Write(p); err != nil {
		return
	}

	err = l.OutputLines()
	return
}

func (l *Logstreamer) Close() error {
	if err := l.Flush(); err != nil {
		return err
	}
	l.buf = bytes.NewBuffer([]byte(""))
	return nil
}

func (l *Logstreamer) Flush() error {
	p := make([]byte, l.buf.Len())
	if _, err := l.buf.Read(p); err != nil {
		return err
	}

	l.out(string(p))
	return nil
}

func (l *Logstreamer) OutputLines() error {
	for {
		line, err := l.buf.ReadString('\n')

		if len(line) > 0 {
			if strings.HasSuffix(line, "\n") {
				l.out(line)
			} else {
				// put back into buffer, it's not a complete line yet
				//  Close() or Flush() have to be used to flush out
				//  the last remaining line if it does not end with a newline
				if _, err := l.buf.WriteString(line); err != nil {
					return err
				}
			}
		}

		if err == io.EOF {
			break
		}

		if err != nil {
			return err
		}
	}

	return nil
}

func (l *Logstreamer) FlushRecord() string {
	buffer := l.persist
	l.persist = ""
	return buffer
}

func (l *Logstreamer) out(str string) {
	if len(str) < 1 {
		return
	}

	if l.record {
		l.persist = l.persist + str
	}

	if l.prefix == "stdout" {
		str = l.colorOkay + l.prefix + l.colorReset + " " + str
	} else if l.prefix == "stderr" {
		str = l.colorFail + l.prefix + l.colorReset + " " + str
	}

	l.Logger.Print(str)
}

// PrettyStdoutWriter wraps an ioWriter so it can add string
// prefixes to every message it writes to stdout.
type PrettyStdoutWriter struct {
	w      io.Writer
	Prefix string
}

var _ io.Writer = (*PrettyStdoutWriter)(nil)

// NewPrettyStdoutWriter returns an instance of PrettyStdoutWriter
func NewPrettyStdoutWriter(prefix string) *PrettyStdoutWriter {
	return &PrettyStdoutWriter{
		w:      os.Stdout,
		Prefix: prefix,
	}
}

func (psw *PrettyStdoutWriter) Write(p []byte) (int, error) {
	str := psw.Prefix + string(p)
	n, err := psw.w.Write([]byte(str))

	if err != nil {
		return n, err
	}

	return len(p), nil
}
