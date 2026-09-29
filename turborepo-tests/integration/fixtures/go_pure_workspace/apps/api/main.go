package main

import (
	"fmt"
	"os"

	"example.com/lib"
)

func main() {
	lib.Greet()
	if len(os.Args) > 1 {
		fmt.Println(os.Args[1])
	}
}
