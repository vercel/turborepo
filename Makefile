
TURBO_VERSION = $(shell head -n 1 version.txt)
TURBO_TAG = $(shell cat version.txt | sed -n '2 p')

turbo: cmd/turbo/version.go cmd/turbo/*.go internal/*/*.go go.mod
	go build ./cmd/turbo

# These tests are for development
test: test-go vet-go

dev: 
	cd app && yarn dev

studio:
	cd app && yarn prisma:studio

ewatch: | scripts/node_modules
	nodemon --exec "make turbo && make e2e" -e .ts,.go
# # These tests are for release ("test-wasm" is not included in "test" because it's pretty slow)
# test-all:
# 	make -j6 test-go vet-go verify-source-map end-to-end-tests js-api-tests test-wasm

# # This includes tests of some 3rd-party libraries, which can be very slow
# test-extra: test-all test-sucrase test-esprima test-rollup

check-go-version:
	@go version | grep ' go1\.16 ' || (echo 'Please install Go version 1.16.0' && false)


# This "TURBO_RACE" variable exists at the request of a user on GitHub who
# wants to run "make test" on an unsupported version of macOS (version 10.9).
# Go's race detector does not run correctly on that version. With this flag
# you can run "TURBO_RACE= make test" to disable the race detector.
TURBO_RACE ?= -race

test-go:
	go test $(TURBO_RACE) ./internal/...

vet-go:
	go vet ./cmd/... ./internal/...

fmt-go:
	go fmt ./cmd/... ./internal/...

# verify-source-map: | scripts/node_modules
# 	node scripts/verify-source-map.js

scripts: | scripts/node_modules
	cd scripts && yarn install

e2e: | scripts/node_modules
	cd scripts && yarn uvu -r esbuild-register e2e

# js-api-tests: | scripts/node_modules
# 	node scripts/js-api-tests.js

cmd/turbo/version.go: version.txt
	# Update this atomically to avoid issues with this being overwritten during use
	node -e 'console.log(`package main\n\nconst turboVersion = "$(TURBO_VERSION)"`)' > cmd/turbo/version.go.txt
	mv cmd/turbo/version.go.txt cmd/turbo/version.go

platform-all: cmd/turbo/version.go
	make -j13 \
		platform-windows \
		platform-windows-32 \
		platform-darwin \
		platform-darwin-arm64 \
		platform-freebsd \
		platform-freebsd-arm64 \
		platform-linux \
		platform-linux-32 \
		platform-linux-arm \
		platform-linux-arm64 \
		platform-linux-mips64le \
		platform-linux-ppc64le \
		platform-neutral


platform-windows:
	cd npm/turbo-windows-64 && npm version "$(TURBO_VERSION)" --allow-same-version
	GOOS=windows GOARCH=amd64 go build "-ldflags=-s -w" -o npm/turbo-windows-64/turbo.exe ./cmd/turbo

platform-windows-32:
	cd npm/turbo-windows-32 && npm version "$(TURBO_VERSION)" --allow-same-version
	GOOS=windows GOARCH=386 go build "-ldflags=-s -w" -o npm/turbo-windows-32/turbo.exe ./cmd/turbo

platform-unixlike:
	test -n "$(GOOS)" && test -n "$(GOARCH)" && test -n "$(NPMDIR)"
	mkdir -p "$(NPMDIR)/bin"
	cd "$(NPMDIR)" && npm version "$(TURBO_VERSION)" --allow-same-version
	GOOS="$(GOOS)" GOARCH="$(GOARCH)" go build "-ldflags=-s -w" -o "$(NPMDIR)/bin/turbo" ./cmd/turbo

platform-darwin:
	make GOOS=darwin GOARCH=amd64 NPMDIR=npm/turbo-darwin-64 platform-unixlike

platform-darwin-arm64:
	make GOOS=darwin GOARCH=arm64 NPMDIR=npm/turbo-darwin-arm64 platform-unixlike

platform-freebsd:
	make GOOS=freebsd GOARCH=amd64 NPMDIR=npm/turbo-freebsd-64 platform-unixlike

platform-freebsd-arm64:
	make GOOS=freebsd GOARCH=arm64 NPMDIR=npm/turbo-freebsd-arm64 platform-unixlike

platform-linux:
	make GOOS=linux GOARCH=amd64 NPMDIR=npm/turbo-linux-64 platform-unixlike

platform-linux-32:
	make GOOS=linux GOARCH=386 NPMDIR=npm/turbo-linux-32 platform-unixlike

platform-linux-arm:
	make GOOS=linux GOARCH=arm NPMDIR=npm/turbo-linux-arm platform-unixlike

platform-linux-arm64:
	make GOOS=linux GOARCH=arm64 NPMDIR=npm/turbo-linux-arm64 platform-unixlike

platform-linux-mips64le:
	make GOOS=linux GOARCH=mips64le NPMDIR=npm/turbo-linux-mips64le platform-unixlike

platform-linux-ppc64le:
	make GOOS=linux GOARCH=ppc64le NPMDIR=npm/turbo-linux-ppc64le platform-unixlike

platform-neutral: | turbo
	cd npm/turbo-install && npm version "$(TURBO_VERSION)" --allow-same-version

test-prepublish:
	rm -rf demo/turbo
	make demo/turbo
	make turbo
	make -j3 bench/turbo test-go vet-go e2e

test-otp:
	test -n "$(OTP)" && echo publish --otp="$(OTP)"

publish-all: cmd/turbo/version.go
	# @test main = "`git rev-parse --abbrev-ref HEAD`" || (echo "Refusing to publish from non-master branch `git rev-parse --abbrev-ref HEAD`" && false)
	@echo "Checking for unpushed commits..." && git fetch
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)
	rm -fr npm && git checkout npm
	@echo Enter one-time password:
	make publish-windows
	make publish-windows-32
	make publish-freebsd
	make publish-freebsd-arm64
	@echo Enter one-time password:
	make -j4 \
		publish-darwin \
		publish-darwin-arm64 \
		publish-linux \
		publish-linux-32
	@echo Enter one-time password:
	make -j4 \
		publish-linux-arm \
		publish-linux-arm64 \
		publish-linux-mips64le \
		publish-linux-ppc64le
	# Do these last to avoid race conditions
	@echo Confirm release:
	make publish-neutral
	git commit -am "publish $(TURBO_VERSION) to registry"
	git tag "v$(TURBO_VERSION)"
	git push origin main "v$(TURBO_VERSION)"

publish-windows: platform-windows
	make test && cd npm/turbo-windows-64 && npm publish --tag $(TURBO_TAG)

publish-windows-32: platform-windows-32
	make test && cd npm/turbo-windows-32 && npm publish --tag $(TURBO_TAG)

publish-darwin: platform-darwin
	make test && cd npm/turbo-darwin-64 && npm publish --tag $(TURBO_TAG)

publish-darwin-arm64: platform-darwin-arm64
	make test && cd npm/turbo-darwin-arm64 && npm publish --tag $(TURBO_TAG)

publish-freebsd: platform-freebsd
	make test && cd npm/turbo-freebsd-64 && npm publish --tag $(TURBO_TAG)

publish-freebsd-arm64: platform-freebsd-arm64
	make test && cd npm/turbo-freebsd-arm64 && npm publish --tag $(TURBO_TAG)

publish-linux: platform-linux
	make test && cd npm/turbo-linux-64 && npm publish --tag $(TURBO_TAG)

publish-linux-32: platform-linux-32
	make test && cd npm/turbo-linux-32 && npm publish --tag $(TURBO_TAG)

publish-linux-arm: platform-linux-arm
	make test && cd npm/turbo-linux-arm && npm publish --tag $(TURBO_TAG)

publish-linux-arm64: platform-linux-arm64
	make test && cd npm/turbo-linux-arm64 && npm publish --tag $(TURBO_TAG)

publish-linux-mips64le: platform-linux-mips64le
	make test && cd npm/turbo-linux-mips64le && npm publish --tag $(TURBO_TAG)

publish-linux-ppc64le: platform-linux-ppc64le
	make test && cd npm/turbo-linux-ppc64le && npm publish --tag $(TURBO_TAG)

publish-neutral: platform-neutral
	make test && cd npm/turbo-install && npm publish --tag $(TURBO_TAG)

scripts/node_modules:
	cd scripts && yarn

demo/lage: | scripts/node_modules
	node scripts/generate.mjs lage

demo/lerna: | scripts/node_modules
	node scripts/generate.mjs lerna

demo/nx: | scripts/node_modules
	node scripts/generate.mjs nx

demo/turbo: | scripts/node_modules
	node scripts/generate.mjs turbo

bench/lerna: demo/lerna
	cd demo/lerna && node_modules/.bin/lerna run build

bench/lage: demo/lage
	cd demo/lage && node_modules/.bin/lage build

bench/nx: demo/nx
	cd demo/nx && node_modules/.bin/nx run-many --target=build --all

bench/turbo: demo/turbo
	cd demo/turbo && ../../turbo run test --force

bench/turbo-new: demo/turbo
	cd demo/turbo && ../../turbo-new run build test 

bench: bench/lerna bench/lage bench/nx bench/turbo

clean:
	rm -f turbo
	rm -rf npm/turbo-darwin-64/bin/turbo
	rm -rf npm/turbo-darwin-arm64/bin/turbo
	rm -rf npm/turbo-freebsd-64/bin/turbo
	rm -rf npm/turbo-freebsd-arm64/bin/turbo
	rm -rf npm/turbo-linux-32/bin/turbo
	rm -rf npm/turbo-linux-64/bin/turbo
	rm -rf npm/turbo-linux-arm/bin/turbo
	rm -rf npm/turbo-linux-arm64/bin/turbo
	rm -rf npm/turbo-linux-mips64le/bin/turbo
	rm -rf npm/turbo-linux-ppc64le/bin/turbo
	rm -rf npm/turbo-windows-32/turbo.exe
	rm -rf npm/turbo-windows-64/turbo.exe
	rm -rf playground/*/dist
	rm -rf playground/*/.next
	rm -rf playground/*/turbo
	rm -rf playground/*/.turbo
	rm -rf playground/*/node_modules
	rm -rf packages/*/dist
	rm -rf packages/*/.turbo
	rm -rf packages/*/turbo
	rm -rf packages/*/node_modules
	rm -rf docs/.turbo
	rm -rf docs/.next
	rm -rf docs/node_modules
	rm -rf node_modules

	rm -rf demo
	go clean -testcache ./internal/...

platform-ts:
	cd packages/turbo-core && npm version "$(TURBO_VERSION)" --allow-same-version
	cd packages/turbo-core && rm -rf dist && yarn lint && yarn compile

publish-ts: platform-ts
	cd packages/turbo-core && npm publish --tag $(TURBO_TAG)

publish-js: cmd/turbo/version.go
	@test main = "`git rev-parse --abbrev-ref HEAD`" || (echo "Refusing to publish from non-master branch `git rev-parse --abbrev-ref HEAD`" && false)
	@echo "Checking for unpushed commits..." && git fetch
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)
	rm -fr npm && git checkout npm
	@echo Confirm release:
	make publish-ts
	git commit -am "publish $(TURBO_VERSION) to registry"
	git tag "v$(TURBO_VERSION)"
	git push origin main "v$(TURBO_VERSION)"