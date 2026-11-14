// A Go plugin loaded with plugin.Open (built with -buildmode=plugin).
package main

import (
	"fmt"
	"plugin"
)

func main() {
	p, err := plugin.Open("plugins/greeter.so")
	if err != nil {
		panic(err)
	}
	sym, _ := p.Lookup("Name")
	fmt.Println(*sym.(*string))
}
