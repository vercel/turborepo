package ui

import (
	"bytes"
	"io"

	"github.com/mitchellh/cli"
)

type QueuedUi struct {
	out io.Writer
	err io.Writer
	in  io.Reader

	OutBuffer *bytes.Buffer
	ErrBuffer *bytes.Buffer

	ui cli.Ui
}

func (u *QueuedUi) Ask(query string) (string, error) {
	return u.ui.Ask(query)
}

func (u *QueuedUi) AskSecret(query string) (string, error) {
	return u.ui.AskSecret(query)
}

func (u *QueuedUi) Error(message string) {
	u.ui.Error(message)
}

func (u *QueuedUi) Info(message string) {
	u.ui.Info(message)
}

func (u *QueuedUi) Output(message string) {
	u.ui.Output(message)
}

func (u *QueuedUi) Warn(message string) {
	u.ui.Warn(message)
}

func (u *QueuedUi) WriteOutput() {
	u.out.Write(u.OutBuffer.Bytes())
	u.err.Write(u.ErrBuffer.Bytes())
}
