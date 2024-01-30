// Copyright (c) 2013 Kevin van Zonneveld <kevin@vanzonneveld.net>. All rights reserved.
// Source: https://github.com/kvz/logstreamer
// SPDX-License-Identifier: MIT
package logstreamer

import (
	"bufio"
	"bytes"
	"fmt"
	"log"
	"os"
	"os/exec"
	"strings"
	"testing"
)

func TestLogstreamerOk(t *testing.T) {
	// Create a logger (your app probably already has one)
	logger := log.New(os.Stdout, "--> ", log.Ldate|log.Ltime)

	// Setup a streamer that we'll pipe cmd.Stdout to
	logStreamerOut := NewLogstreamer(logger, "stdout", false)
	defer logStreamerOut.Close()
	// Setup a streamer that we'll pipe cmd.Stderr to.
	// We want to record/buffer anything that's written to this (3rd argument true)
	logStreamerErr := NewLogstreamer(logger, "stderr", true)
	defer logStreamerErr.Close()

	// Execute something that succeeds
	cmd := exec.Command(
		"ls",
		"-al",
	)
	cmd.Stderr = logStreamerErr
	cmd.Stdout = logStreamerOut

	// Reset any error we recorded
	logStreamerErr.FlushRecord()

	// Execute command
	err := cmd.Start()

	// Failed to spawn?
	if err != nil {
		t.Fatal("ERROR could not spawn command.", err.Error())
	}

	// Failed to execute?
	err = cmd.Wait()
	if err != nil {
		t.Fatal("ERROR command finished with error. ", err.Error(), logStreamerErr.FlushRecord())
	}
}

func TestLogstreamerErr(t *testing.T) {
	// Create a logger (your app probably already has one)
	logger := log.New(os.Stdout, "--> ", log.Ldate|log.Ltime)

	// Setup a streamer that we'll pipe cmd.Stdout to
	logStreamerOut := NewLogstreamer(logger, "stdout", false)
	defer logStreamerOut.Close()
	// Setup a streamer that we'll pipe cmd.Stderr to.
	// We want to record/buffer anything that's written to this (3rd argument true)
	logStreamerErr := NewLogstreamer(logger, "stderr", true)
	defer logStreamerErr.Close()

	// Execute something that succeeds
	cmd := exec.Command(
		"ls",
		"nonexisting",
	)
	cmd.Stderr = logStreamerErr
	cmd.Stdout = logStreamerOut

	// Reset any error we recorded
	logStreamerErr.FlushRecord()

	// Execute command
	err := cmd.Start()

	// Failed to spawn?
	if err != nil {
		logger.Print("ERROR could not spawn command. ")
	}

	// Failed to execute?
	err = cmd.Wait()
	if err != nil {
		fmt.Printf("Good. command finished with %s. %s. \n", err.Error(), logStreamerErr.FlushRecord())
	} else {
		t.Fatal("This command should have failed")
	}
}

func TestLogstreamerFlush(t *testing.T) {
	const text = "Text without newline"

	var buffer bytes.Buffer
	byteWriter := bufio.NewWriter(&buffer)

	logger := log.New(byteWriter, "", 0)
	logStreamerOut := NewLogstreamer(logger, "", false)
	defer logStreamerOut.Close()

	logStreamerOut.Write([]byte(text))
	logStreamerOut.Flush()
	byteWriter.Flush()

	s := strings.TrimSpace(buffer.String())

	if s != text {
		t.Fatalf("Expected '%s', got '%s'.", text, s)
	}
}
