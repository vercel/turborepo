package ui

import (
	"fmt"
	"io"
	"math"
	"os"
	"regexp"
	"strings"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/mitchellh/cli"
	"github.com/vercel/turbo/cli/internal/ci"
)

const ansiEscapeStr = "[\u001B\u009B][[\\]()#;?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[a-zA-Z\\d]*)*)?\u0007)|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PRZcf-ntqry=><~]))"

// IsTTY is true when stdout appears to be a tty
var IsTTY = isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())

// IsCI is true when we appear to be running in a non-interactive context.
var IsCI = !IsTTY || ci.IsCi()
var gray = color.New(color.Faint)
var bold = color.New(color.Bold)
var ERROR_PREFIX = color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
var WARNING_PREFIX = color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")

// InfoPrefix is a colored string for warning level log messages
var InfoPrefix = color.New(color.Bold, color.FgWhite, color.ReverseVideo).Sprint(" INFO ")

var ansiRegex = regexp.MustCompile(ansiEscapeStr)

// Dim prints out dimmed text
func Dim(str string) string {
	return gray.Sprint(str)
}

func Bold(str string) string {
	return bold.Sprint(str)
}

// Adapted from go-rainbow
// Copyright (c) 2017 Raphael Amorim
// Source: https://github.com/raphamorim/go-rainbow
// SPDX-License-Identifier: MIT
func rgb(i int) (int, int, int) {
	var f = 0.275

	return int(math.Sin(f*float64(i)+4*math.Pi/3)*127 + 128),
		// int(math.Sin(f*float64(i)+2*math.Pi/3)*127 + 128),
		int(45),
		int(math.Sin(f*float64(i)+0)*127 + 128)
}

// Rainbow function returns a formated colorized string ready to print it to the shell/terminal
//
// Adapted from go-rainbow
// Copyright (c) 2017 Raphael Amorim
// Source: https://github.com/raphamorim/go-rainbow
// SPDX-License-Identifier: MIT
func Rainbow(text string) string {
	var rainbowStr []string
	for index, value := range text {
		r, g, b := rgb(index)
		str := fmt.Sprintf("\033[1m\033[38;2;%d;%d;%dm%c\033[0m\033[0;1m", r, g, b, value)
		rainbowStr = append(rainbowStr, str)
	}

	return strings.Join(rainbowStr, "")
}

type stripAnsiWriter struct {
	wrappedWriter io.Writer
}

func (into *stripAnsiWriter) Write(p []byte) (int, error) {
	n, err := into.wrappedWriter.Write(ansiRegex.ReplaceAll(p, []byte{}))
	if err != nil {
		// The number of bytes returned here isn't directly related to the input bytes
		// if ansi color codes were being stripped out, but we are counting on Stdout.Write
		// not failing under typical operation as well.
		return n, err
	}

	// Write must return a non-nil error if it returns n < len(p). Consequently, if the
	// wrappedWrite.Write call succeeded we will return len(p) as the number of bytes
	// written.
	return len(p), nil
}

// Default returns the default colored ui
func Default() *cli.ColoredUi {
	return BuildColoredUi(ColorModeUndefined)
}

func BuildColoredUi(colorMode ColorMode) *cli.ColoredUi {
	colorMode = applyColorMode(colorMode)

	var outWriter, errWriter io.Writer

	if colorMode == ColorModeSuppressed {
		outWriter = &stripAnsiWriter{wrappedWriter: os.Stdout}
		errWriter = &stripAnsiWriter{wrappedWriter: os.Stderr}
	} else {
		outWriter = os.Stdout
		errWriter = os.Stderr
	}

	return &cli.ColoredUi{
		Ui: &cli.BasicUi{
			Reader:      os.Stdin,
			Writer:      outWriter,
			ErrorWriter: errWriter,
		},
		OutputColor: cli.UiColorNone,
		InfoColor:   cli.UiColorNone,
		WarnColor:   cli.UiColor{Code: int(color.FgYellow), Bold: false},
		ErrorColor:  cli.UiColorRed,
	}
}
