package ui

import (
	"fmt"
	"io"
	"math"
	"os"
	"strings"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
)

const ESC = 27

var IsTTY = isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
var IsCI = os.Getenv("CI") == "true" || os.Getenv("BUILD_NUMBER") == "true" || os.Getenv("TEAMCITY_VERSION") != ""
var gray = color.New(color.Faint)
var bold = color.New(color.Bold)
var ERROR_PREFIX = color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")
var WARNING_PREFIX = color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")

// clear the line and move the cursor up
var clear = fmt.Sprintf("%c[%dA%c[2K", ESC, 1, ESC)

func ClearLines(writer io.Writer, count int) {
	_, _ = fmt.Fprint(writer, strings.Repeat(clear, count))
}

// Dim prints out dimmed text
func Dim(str string) string {
	return gray.Sprint(str)
}

func Bold(str string) string {
	return bold.Sprint(str)
}

func Warn(str string) string {
	return fmt.Sprintf("%s %s", WARNING_PREFIX, color.YellowString(str))
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
