module example.com/api

go 1.22

require (
	example.com/lib v0.0.0
	example.net/message v0.0.0
)

replace example.com/lib => ../../packages/lib

replace example.net/message => ../../third_party/message
