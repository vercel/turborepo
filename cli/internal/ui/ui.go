package ui

import (
	"fmt"
	"math"
	"os"
	"regexp"
	"strings"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	"github.com/mitchellh/cli"
)

const ansiEscapeStr = "[\u001B\u009B][[\\]()#;?]*(?:(?:(?:[a-zA-Z\\d]*(?:;[a-zA-Z\\d]*)*)?\u0007)|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PRZcf-ntqry=><~]))"

var IsTTY = isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
var IsCI = os.Getenv("CI") == "true" || os.Getenv("BUILD_NUMBER") == "true" || os.Getenv("TEAMCITY_VERSION") != ""
var gray = color.New(color.Faint)
var bold = color.New(color.Bold)
var ERROR_PREFIX = color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
var WARNING_PREFIX = color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")

var ansiRegex = regexp.MustCompile(ansiEscapeStr)

func StripAnsi(str string) string {
	if !IsTTY {
		return ansiRegex.ReplaceAllString(str, "")
	}
	return str
}

// Dim prints out dimmed text
func Dim(str string) string {
	return gray.Sprint(str)
}

func Bold(str string) string {
	return bold.Sprint(str)
}

func rgb(i int) (int, int, int) {
	var f = 0.275

	return int(math.Sin(f*float64(i)+4*math.Pi/3)*127 + 128),
		// int(math.Sin(f*float64(i)+2*math.Pi/3)*127 + 128),
		int(45),
		int(math.Sin(f*float64(i)+0)*127 + 128)
}

// Rainbow function returns a formated colorized string ready to print it to the shell/terminal
func Rainbow(text string) string {
	if !IsTTY {
		return text
	}
	var rainbowStr []string
	for index, value := range text {
		r, g, b := rgb(index)
		str := fmt.Sprintf("\033[1m\033[38;2;%d;%d;%dm%c\033[0m\033[0;1m", r, g, b, value)
		rainbowStr = append(rainbowStr, str)
	}

	return strings.Join(rainbowStr, "")
}

// Default returns the default colored ui
func Default() *cli.ColoredUi {
	return &cli.ColoredUi{
		Ui: &cli.BasicUi{
			Reader:      os.Stdin,
			Writer:      os.Stdout,
			ErrorWriter: os.Stderr,
		},
		OutputColor: cli.UiColorNone,
		InfoColor:   cli.UiColorNone,
		WarnColor:   cli.UiColorYellow,
		ErrorColor:  cli.UiColorRed,
	}
}
