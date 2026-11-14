// time.LoadLocation reads the system zoneinfo database unless time/tzdata is imported.
package main

import (
	"fmt"
	"time"
)

func main() {
	loc, err := time.LoadLocation("Europe/Paris")
	fmt.Println(loc, err)
}
