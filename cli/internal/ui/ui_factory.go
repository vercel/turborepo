package ui

import (
	"bytes"
	"io"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

// UiFactory provides an interface for creating cli.Ui instances from input, output and error IOs
type UiFactory interface {
	Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui
}

// BasicUiFactory provides a method for creating a cli.BasicUi from input, output and error IOs
type BasicUiFactory struct {
}

// Build builds a cli.BasicUi from input, output and error IOs
func (factory *BasicUiFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &cli.BasicUi{
		Reader:      in,
		Writer:      out,
		ErrorWriter: err,
	}
}

// ColoredUiFactory provides a method for creating a cli.ColoredUi from input, output and error IOs
type ColoredUiFactory struct {
	ColorMode ColorMode
	Base      UiFactory
}

// Build builds a cli.ColoredUi from input, output and error IOs
func (factory *ColoredUiFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	factory.ColorMode = applyColorMode(factory.ColorMode)

	var outWriter, errWriter io.Writer

	if factory.ColorMode == ColorModeSuppressed {
		outWriter = &stripAnsiWriter{wrappedWriter: out}
		errWriter = &stripAnsiWriter{wrappedWriter: err}
	} else {
		outWriter = out
		errWriter = err
	}

	return &cli.ColoredUi{
		Ui:          factory.Base.Build(in, outWriter, errWriter),
		OutputColor: cli.UiColorNone,
		InfoColor:   cli.UiColorNone,
		WarnColor:   cli.UiColor{Code: int(color.FgYellow), Bold: false},
		ErrorColor:  cli.UiColorRed,
	}
}

// ConcurrentUiFactory provides a method for creating a cli.ConcurrentUi from input, output and error IOs
type ConcurrentUiFactory struct {
	Base UiFactory
}

// Build builds a cli.ConcurrentUi from input, output and error IOs
func (factory *ConcurrentUiFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &cli.ConcurrentUi{
		Ui: factory.Base.Build(in, out, err),
	}
}

// PrefixedUiFactory provides a method for creating a cli.PrefixedUi from input, output and error IOs
type PrefixedUiFactory struct {
	Base UiFactory

	AskPrefix       string
	AskSecretPrefix string
	OutputPrefix    string
	InfoPrefix      string
	ErrorPrefix     string
	WarnPrefix      string
}

// Build builds a cli.PrefixedUi from input, output and error IOs
func (factory *PrefixedUiFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &cli.PrefixedUi{
		AskPrefix:       factory.AskPrefix,
		AskSecretPrefix: factory.AskSecretPrefix,
		OutputPrefix:    factory.OutputPrefix,
		InfoPrefix:      factory.InfoPrefix,
		ErrorPrefix:     factory.ErrorPrefix,
		WarnPrefix:      factory.WarnPrefix,
		Ui:              factory.Base.Build(in, out, err),
	}
}

type QueuedUiFactory struct {
	Base UiFactory
}

func (factory *QueuedUiFactory) Build(in io.Reader, out io.Writer, err io.Writer) *QueuedUi {
	outBuf := &bytes.Buffer{}
	errBuf := &bytes.Buffer{}

	return &QueuedUi{
		out: out,
		err: err,
		in:  in,

		OutBuffer: outBuf,
		ErrBuffer: errBuf,
		ui:        factory.Base.Build(in, io.Writer(outBuf), io.Writer(errBuf)),
	}
}
