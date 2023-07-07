package ui

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strings"

	"github.com/bgentry/speakeasy"
	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/mitchellh/cli"
)

// Factory provides an interface for creating cli.Ui instances from input, output and error IOs
type Factory interface {
	Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui
}

// BasicUIFactory provides a method for creating a cli.BasicUi from input, output and error IOs
type BasicUIFactory struct {
}

// basicUI is an implementation of Ui that just outputs to the given
// writer. This UI is not threadsafe by default, but you can wrap it
// in a ConcurrentUi to make it safe.
//
// Inlined from cli.Ui to fuse newlines to lines being logged. This is
// probably not the optimal way to do it, but it works for now.
type basicUI struct {
	Reader      io.Reader
	Writer      io.Writer
	ErrorWriter io.Writer
}

// Ask implements ui.Cli.Ask for BasicUi
func (u *basicUI) Ask(query string) (string, error) {
	return u.ask(query, false)
}

// AskSecret implements ui.Cli.AskSecret for BasicUi
func (u *basicUI) AskSecret(query string) (string, error) {
	return u.ask(query, true)
}

func (u *basicUI) ask(query string, secret bool) (string, error) {
	if _, err := fmt.Fprint(u.Writer, query+" "); err != nil {
		return "", err
	}

	// Register for interrupts so that we can catch it and immediately
	// return...
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, os.Interrupt)
	defer signal.Stop(sigCh)

	// Ask for input in a go-routine so that we can ignore it.
	errCh := make(chan error, 1)
	lineCh := make(chan string, 1)
	go func() {
		var line string
		var err error
		if secret && isatty.IsTerminal(os.Stdin.Fd()) {
			line, err = speakeasy.Ask("")
		} else {
			r := bufio.NewReader(u.Reader)
			line, err = r.ReadString('\n')
		}
		if err != nil {
			errCh <- err
			return
		}

		lineCh <- strings.TrimRight(line, "\r\n")
	}()

	select {
	case err := <-errCh:
		return "", err
	case line := <-lineCh:
		return line, nil
	case <-sigCh:
		// Print a newline so that any further output starts properly
		// on a new line.
		fmt.Fprintln(u.Writer)

		return "", errors.New("interrupted")
	}
}

// Error implements ui.Cli.Error for BasicUi
func (u *basicUI) Error(message string) {
	w := u.Writer
	if u.ErrorWriter != nil {
		w = u.ErrorWriter
	}

	fmt.Fprintf(w, "%v\n", message)
}

// Info implements ui.Cli.Info for BasicUi
func (u *basicUI) Info(message string) {
	u.Output(message)
}

// Output implements ui.Cli.Output for BasicUi
func (u *basicUI) Output(message string) {
	fmt.Fprintf(u.Writer, "%v\n", message)
}

// Warn implements ui.Cli.Warn for BasicUi
func (u *basicUI) Warn(message string) {
	u.Error(message)
}

// Build builds a cli.BasicUi from input, output and error IOs
func (factory *BasicUIFactory) Build(in io.Reader, out io.Writer, err io.Writer) cli.Ui {
	return &basicUI{
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
