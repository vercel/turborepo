package ui

import (
	"io"

	"github.com/fatih/color"
	"github.com/mitchellh/cli"
)

// Factory provides an interface for creating cli.Ui instances from input, output and error IOs
type Factory interface {
	Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui
}

// BasicUIFactory provides a method for creating a cli.BasicUi from input, output and error IOs
type BasicUIFactory struct {
}

// Build builds a cli.BasicUi from input, output and error IOs
func (factory *BasicUIFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &cli.BasicUi{
		Reader:      in,
		Writer:      out,
		ErrorWriter: err,
	}
}

// ColoredUIFactory provides a method for creating a cli.ColoredUi from input, output and error IOs
type ColoredUIFactory struct {
	ColorMode ColorMode
	Base      Factory
}

// Build builds a cli.ColoredUi from input, output and error IOs
func (factory *ColoredUIFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
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

// ConcurrentUIFactory provides a method for creating a cli.ConcurrentUi from input, output and error IOs
type ConcurrentUIFactory struct {
	Base Factory
}

// Build builds a cli.ConcurrentUi from input, output and error IOs
func (factory *ConcurrentUIFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &cli.ConcurrentUi{
		Ui: factory.Base.Build(in, out, err),
	}
}

// PrefixedUIFactory provides a method for creating a cli.PrefixedUi from input, output and error IOs
type PrefixedUIFactory struct {
	Base            Factory
	AskPrefix       string
	AskSecretPrefix string
	OutputPrefix    string
	InfoPrefix      string
	ErrorPrefix     string
	WarnPrefix      string
}

// Build builds a cli.PrefixedUi from input, output and error IOs
func (factory *PrefixedUIFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
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
