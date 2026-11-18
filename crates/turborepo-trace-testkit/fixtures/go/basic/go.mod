module example.com/basic

go 1.22

require (
	example.com/lib v0.0.0
	github.com/Upper/dep v1.2.0
	golang.org/x/old v0.1.0 // indirect
)

replace example.com/lib => ./lib
