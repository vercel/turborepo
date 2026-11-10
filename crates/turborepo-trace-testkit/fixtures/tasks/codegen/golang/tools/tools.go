//go:build tools

package tools

//go:generate -command gen go run ./gen
//go:generate gen -out ../data/table.go
//go:generate unknown-generator -pkg $GOPACKAGE
