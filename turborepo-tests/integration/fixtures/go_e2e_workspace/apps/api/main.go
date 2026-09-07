package main

import (
	"fmt"

	"example.com/lib"
	"example.net/message"
)

func main() {
	fmt.Println(lib.Value(), message.Value())
}
