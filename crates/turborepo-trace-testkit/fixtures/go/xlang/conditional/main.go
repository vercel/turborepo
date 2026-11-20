// Build tags: one file per OS reads its own configuration.
package main

import "fmt"

func main() {
	fmt.Println(len(readConf()))
}
