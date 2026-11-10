package pkg

import "os"

// Hello reads a runtime file relative to the working directory.
func Hello() { os.ReadFile("hello.txt") }
