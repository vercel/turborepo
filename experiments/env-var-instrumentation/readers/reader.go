package main

import (
	"fmt"
	"os"
)

func main() {
	fmt.Println("MY_SECRET_TOKEN=" + os.Getenv("MY_SECRET_TOKEN"))
}
