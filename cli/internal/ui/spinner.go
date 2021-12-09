package ui

import (
	"fmt"
	"io"
	"os"
	"time"

	"github.com/briandowns/spinner"
)

// Events display settings.
const (
	minCellWidth           = 20  // minimum number of characters in a table's cell.
	tabWidth               = 4   // number of characters in between columns.
	cellPaddingWidth       = 2   // number of padding characters added by default to a cell.
	paddingChar            = ' ' // character in between columns.
	noAdditionalFormatting = 0
	maxCellLength          = 70 // Number of characters we want to display at most in a cell before wrapping it to the next line.
)

// startStopper is the interface to interact with the spinner.
type startStopper interface {
	Start()
	Stop()
}

// Spinner represents an indicator that an asynchronous operation is taking place.
//
// For short operations, less than 4 seconds, display only the spinner with the Start and Stop methods.
// For longer operations, display intermediate progress events using the Events method.
type Spinner struct {
	spin startStopper
}

// NewSpinner returns a spinner that outputs to w.
func NewSpinner(w io.Writer) *Spinner {
	interval := 125 * time.Millisecond
	if os.Getenv("CI") == "true" {
		interval = 30 * time.Second
	}
	s := spinner.New(charset, interval, spinner.WithHiddenCursor(true))
	s.Writer = w
	s.Color("faint")
	return &Spinner{
		spin: s,
	}
}

// Start starts the spinner suffixed with a label.
func (s *Spinner) Start(label string) {
	s.suffix(fmt.Sprintf(" %s", label))
	s.spin.Start()
}

// Stop stops the spinner and replaces it with a label.
func (s *Spinner) Stop(label string) {
	s.finalMSG(fmt.Sprint(label))
	s.spin.Stop()
}

func (s *Spinner) lock() {
	if spinner, ok := s.spin.(*spinner.Spinner); ok {
		spinner.Lock()
	}
}

func (s *Spinner) unlock() {
	if spinner, ok := s.spin.(*spinner.Spinner); ok {
		spinner.Unlock()
	}
}

func (s *Spinner) suffix(label string) {
	s.lock()
	defer s.unlock()
	if spinner, ok := s.spin.(*spinner.Spinner); ok {
		spinner.Suffix = label
	}
}

func (s *Spinner) finalMSG(label string) {
	s.lock()
	defer s.unlock()
	if spinner, ok := s.spin.(*spinner.Spinner); ok {
		spinner.FinalMSG = label
	}
}
