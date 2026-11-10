//go:build linux

package main

import "os"

func readConf() []byte {
	b, _ := os.ReadFile("conf/linux.conf")
	return b
}
