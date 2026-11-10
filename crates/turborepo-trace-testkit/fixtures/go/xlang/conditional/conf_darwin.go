//go:build darwin

package main

import "os"

func readConf() []byte {
	b, _ := os.ReadFile("conf/darwin.conf")
	return b
}
