package runcache

import (
	"fmt"
	"os"
)

func ExamplePrefixedWriter_Write_withPrefixSet() {
	prefixedWriter := prefixedWriter{
		prefix:           "PREFIXED: ",
		underlyingWriter: os.Stdout,
	}

	someLogs := "First line of log.\nSecond line.\n\tThird line a little different\n"
	if _, err := prefixedWriter.Write([]byte(someLogs)); err != nil {
		fmt.Print("Unexpected write error: ", err)
	}

	// Output:
	// PREFIXED: First line of log.
	// PREFIXED: Second line.
	// PREFIXED: 	Third line a little different
}

func ExamplePrefixedWriter_Write_withNotPrefixSet() {
	prefixedWriter := prefixedWriter{
		underlyingWriter: os.Stdout,
	}

	someLogs := "First line of log.\nSecond line.\n\tThird line a little different\n"
	if _, err := prefixedWriter.Write([]byte(someLogs)); err != nil {
		fmt.Print("Unexpected write error: ", err)
	}

	// Output:
	// First line of log.
	// Second line.
	// 	Third line a little different
}
