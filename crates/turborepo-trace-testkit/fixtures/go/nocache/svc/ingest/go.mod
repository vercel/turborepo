module example.test/ingest

go 1.22

require (
	cloud.acme.test/go v0.110.0
	cloud.acme.test/go/storage v1.30.1
	example.test/pipeline v0.0.0
	github.com/acme-test/assert v1.2.0
)

require (
	github.com/acme-test/deep v0.3.0 // indirect
	github.com/acme-test/yaml v1.0.0 // indirect
)

replace example.test/pipeline => ../../pkg/pipeline
