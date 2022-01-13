// https://stackoverflow.com/questions/23558425/how-do-i-get-the-local-ip-address-in-go
package util

import (
	"log"
	"net"
)

// Get preferred outbound ip of this machine
func GetOutboundIP() net.IP {
	conn, err := net.Dial("udp", "8.8.8.8:80")
	if err != nil {
		log.Fatal(err)
	}
	defer conn.Close()

	localAddr := conn.LocalAddr().(*net.UDPAddr)

	return localAddr.IP
}
