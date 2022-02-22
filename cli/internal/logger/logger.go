package logger

import (
	"fmt"

	"github.com/fatih/color"
)

func Output(output string) {
	fmt.Println(output)
}

func Success(prefix string, message string) {
	successPrefix := color.New(color.Bold, color.FgGreen, color.ReverseVideo).Sprint(" SUCCESS ")

	if prefix != "" {
		prefix += ": "
	} else if prefix == "" {
		prefix = " "
	}

	Output(fmt.Sprintf("%s%s%s", successPrefix, prefix, color.GreenString("%s", message)))
}

func Warn(prefix string, err error) {
	warnPrefix := color.New(color.Bold, color.FgYellow, color.ReverseVideo).Sprint(" WARNING ")

	if prefix != "" {
		prefix += ": "
	} else if prefix == "" {
		prefix = " "
	}

	Output(fmt.Sprintf("%s%s%s", warnPrefix, prefix, color.YellowString("%v", err)))
}

func Error(prefix string, err error) {
	errorPrefix := color.New(color.Bold, color.FgRed, color.ReverseVideo).Sprint(" ERROR ")

	if prefix != "" {
		prefix += ": "
	} else if prefix == "" {
		prefix = " "
	}

	Output(fmt.Sprintf("%s%s%s", errorPrefix, prefix, color.RedString("%v", err)))
}
