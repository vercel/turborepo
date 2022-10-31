// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package util

import (
	"fmt"
	"io"
	"os"

	"github.com/vercel/turbo/cli/internal/ui"
)

// initPrintf sets up the replacements used by printf.
func InitPrintf() {
	if !ui.IsTTY {
		replacements = map[string]string{}
	}
}

// printf is used throughout this package to print something to stderr with some
// replacements for pseudo-shell variables for ANSI formatting codes.
func Sprintf(format string, args ...interface{}) string {
	return os.Expand(fmt.Sprintf(format, args...), replace)
}

func Printf(format string, args ...interface{}) {
	fmt.Fprint(os.Stderr, os.Expand(fmt.Sprintf(format, args...), replace))
}

func Fprintf(writer io.Writer, format string, args ...interface{}) {
	fmt.Fprint(writer, os.Expand(fmt.Sprintf(format, args...), replace))
}

func replace(s string) string {
	return replacements[s]
}

// These are the standard set of replacements we use.
var replacements = map[string]string{
	"BOLD":         "\x1b[1m",
	"BOLD_GREY":    "\x1b[30;1m",
	"BOLD_RED":     "\x1b[31;1m",
	"BOLD_GREEN":   "\x1b[32;1m",
	"BOLD_YELLOW":  "\x1b[33;1m",
	"BOLD_BLUE":    "\x1b[34;1m",
	"BOLD_MAGENTA": "\x1b[35;1m",
	"BOLD_CYAN":    "\x1b[36;1m",
	"BOLD_WHITE":   "\x1b[37;1m",
	"UNDERLINE":    "\x1b[4m",
	"GREY":         "\x1b[2m",
	"RED":          "\x1b[31m",
	"GREEN":        "\x1b[32m",
	"YELLOW":       "\x1b[33m",
	"BLUE":         "\x1b[34m",
	"MAGENTA":      "\x1b[35m",
	"CYAN":         "\x1b[36m",
	"WHITE":        "\x1b[37m",
	"WHITE_ON_RED": "\x1b[37;41;1m",
	"RED_NO_BG":    "\x1b[31;49;1m",
	"RESET":        "\x1b[0m",
	"ERASE_AFTER":  "\x1b[K",
	"CLEAR_END":    "\x1b[0J",
}
