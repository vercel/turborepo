TURBO_VERSION = $(shell cat ../version.txt | sed -n '1 p')
TURBO_TAG = $(shell cat ../version.txt | sed -n '2 p')

EXT :=
ifeq ($(OS),Windows_NT)
	UNAME := Windows
	EXT = .exe
else
	UNAME := $(shell uname -s)
endif

GOARCH:=$(shell go env GOARCH | xargs)
GOOS:=$(shell go env GOOS | xargs)

# Strip debug info
GO_FLAGS += "-ldflags=-s -w"

# Avoid embedding the build path in the executable for more reproducible builds
GO_FLAGS += -trimpath

CLI_DIR = $(shell pwd)

# allow opting in to the rust codepaths
GO_TAG ?= rust

GO_FILES = $(shell find . -name "*.go")
SRC_FILES = $(shell find . -name "*.go" | grep -v "_test.go")
GENERATED_FILES = internal/turbodprotocol/turbod.pb.go internal/turbodprotocol/turbod_grpc.pb.go

# We do not set go-turbo as a dependency because the Rust build.rs
# script will call it for us and copy over the binary
turbo:
	cargo build -p turbo

turbo-prod:
	cargo build --release --manifest-path ../crates/turborepo/Cargo.toml

go-turbo$(EXT): $(GENERATED_FILES) $(SRC_FILES) go.mod turborepo-ffi-install
	CGO_ENABLED=1 go build -tags $(GO_TAG) -o go-turbo$(EXT) ./cmd/turbo


.PHONY: turborepo-ffi-install
turborepo-ffi-install: turborepo-ffi turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/debug/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_$(GOOS)_$(GOARCH).a

.PHONY: turborepo-ffi
turborepo-ffi:
	cd ../crates/turborepo-ffi && cargo build --target-dir ./target

.PHONY: turborepo-ffi-copy-bindings
turborepo-ffi-copy-bindings:
	cp ../crates/turborepo-ffi/bindings.h ./internal/ffi/bindings.h

#
# ffi cross compiling
#
# these targets are used to build the ffi library for each platform
# when doing a release. they _may_ work on your local machine, but
# they're not intended to be used for development.
#

.PHONY: turborepo-ffi-install-windows-amd64
turborepo-ffi-install-windows-amd64: turborepo-ffi-windows-amd64 turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/x86_64-pc-windows-gnu/release/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_windows_amd64.a

.PHONY: turborepo-ffi-install-darwin-arm64
turborepo-ffi-install-darwin-arm64: turborepo-ffi-darwin-arm64 turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/aarch64-apple-darwin/release/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_darwin_arm64.a

.PHONY: turborepo-ffi-install-darwin-amd64
turborepo-ffi-install-darwin-amd64: turborepo-ffi-darwin-amd64 turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/x86_64-apple-darwin/release/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_darwin_amd64.a

.PHONY: turborepo-ffi-install-linux-arm64
turborepo-ffi-install-linux-arm64: turborepo-ffi-linux-arm64 turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/aarch64-unknown-linux-musl/release/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_linux_arm64.a

.PHONY: turborepo-ffi-install-linux-amd64
turborepo-ffi-install-linux-amd64: turborepo-ffi-linux-amd64 turborepo-ffi-copy-bindings
	cp ../crates/turborepo-ffi/target/x86_64-unknown-linux-musl/release/libturborepo_ffi.a ./internal/ffi/libturborepo_ffi_linux_amd64.a

.PHONY: turborepo-ffi-windows-amd64
turborepo-ffi-windows-amd64:
	cd ../crates/turborepo-ffi && cargo build --release --target-dir ./target --target x86_64-pc-windows-gnu

.PHONY: turborepo-ffi-darwin-arm64
turborepo-ffi-darwin-arm64:
	cd ../crates/turborepo-ffi && cargo build --release --target-dir ./target --target aarch64-apple-darwin

.PHONY: turborepo-ffi-darwin-amd64
turborepo-ffi-darwin-amd64:
	cd ../crates/turborepo-ffi && cargo build --release --target-dir ./target --target x86_64-apple-darwin

.PHONY: turborepo-ffi-linux-arm64
turborepo-ffi-linux-arm64:
	cd ../crates/turborepo-ffi && CC="zig cc -target aarch64-linux-musl" cargo build --release --target-dir ./target --target aarch64-unknown-linux-musl

.PHONY: turborepo-ffi-linux-amd64
turborepo-ffi-linux-amd64:
	cd ../crates/turborepo-ffi && CC="zig cc -target x86_64-linux-musl" cargo build --release --target-dir ./target --target x86_64-unknown-linux-musl

#
# end
#

.PHONY: turborepo-ffi-proto
turborepo-ffi-proto:
	protoc -I../crates/ ../crates/turborepo-ffi/messages.proto --go_out=./internal/

protoc: internal/turbodprotocol/turbod.proto
	protoc --go_out=. --go_opt=paths=source_relative \
		--go-grpc_out=. --go-grpc_opt=paths=source_relative \
		internal/turbodprotocol/turbod.proto

$(GENERATED_FILES): internal/turbodprotocol/turbod.proto
	make protoc

compile-protos: $(GENERATED_FILES)

ewatch: scripts/...
	nodemon --exec "make e2e" -e .ts,.go

check-go-version:
	@go version | grep ' go1\.18\.0 ' || (echo 'Please install Go version 1.18.0' && false)

# This "TURBO_RACE" variable exists at the request of a user on GitHub who
# wants to run "make test-go" on an unsupported version of macOS (version 10.9).
# Go's race detector does not run correctly on that version. With this flag
# you can run "TURBO_RACE= make test-go" to disable the race detector.
TURBO_RACE ?= -race

ifeq ($(UNAME), Windows)
	TURBO_RACE=
endif

clean-go:
	go clean -testcache -r

test-go: $(GENERATED_FILES) $(GO_FILES) go.mod go.sum turborepo-ffi-install
	go test $(TURBO_RACE) -tags $(GO_TAG) ./...

# protos need to be compiled before linting, since linting needs to pick up
# some types from the generated code
lint-go: $(GENERATED_FILES) $(GO_FILES) go.mod go.sum
	golangci-lint run --new-from-rev=main

fmt-go: $(GO_FILES) go.mod go.sum
	go fmt ./...

install: | ./package.json
	pnpm install --filter=cli

corepack:
	which corepack || npm install -g corepack@latest
	corepack enable

e2e: corepack install turbo
	node -r esbuild-register scripts/e2e/e2e.ts

# Expects turbo to be built and up to date
# Only should be used by CI
e2e-prebuilt: corepack install
	node -r esbuild-register scripts/e2e/e2e.ts

cmd/turbo/version.go: ../version.txt
	# Update this atomically to avoid issues with this being overwritten during use
	node -e 'console.log(`package main\n\nconst turboVersion = "$(TURBO_VERSION)"`)' > cmd/turbo/version.go.txt
	mv cmd/turbo/version.go.txt cmd/turbo/version.go

build: install
	cd $(CLI_DIR)/../ && pnpm install --filter=create-turbo && pnpm turbo build --filter=create-turbo...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/codemod && pnpm turbo build --filter=@turbo/codemod...
	cd $(CLI_DIR)/../ && pnpm install --filter=turbo-ignore && pnpm turbo build --filter=turbo-ignore...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/workspaces && pnpm turbo build --filter=@turbo/workspaces...
	cd $(CLI_DIR)/../ && pnpm install --filter=eslint-plugin-turbo && pnpm turbo build --filter=eslint-plugin-turbo...
	cd $(CLI_DIR)/../ && pnpm install --filter=eslint-config-turbo && pnpm turbo build --filter=eslint-config-turbo...

.PHONY: prepublish
prepublish: compile-protos cmd/turbo/version.go
	make -j3 bench/turbo test-go

.PHONY: publish-turbo-cross
publish-turbo-cross: prepublish
	goreleaser release --rm-dist -f cross-release.yml

.PHONY: publish-turbo-darwin
publish-turbo-darwin: prepublish
	goreleaser release --rm-dist -f darwin-release.yml

.PHONY: snapshot-turbo-cross
snapshot-turbo-cross:
	goreleaser release --snapshot --rm-dist -f cross-release.yml

.PHONY: snapshot-turbo-darwin
snapshot-turbo-darwin:
	goreleaser release --snapshot --rm-dist -f darwin-release.yml

.PHONY: snapshot-lib-turbo-darwin
snapshot-lib-turbo-darwin:
	goreleaser release --snapshot --rm-dist -f darwin-lib.yml

.PHONY: snapshot-lib-turbo-cross
snapshot-lib-turbo-cross:
	goreleaser release --snapshot --rm-dist -f cross-lib.yml

.PHONY: build-lib-turbo-darwin
build-lib-turbo-darwin:
	goreleaser release --rm-dist -f darwin-lib.yml

.PHONY: build-go-turbo-darwin
build-go-turbo-darwin:
	goreleaser release --rm-dist -f darwin-release.yml

.PHONY: build-go-turbo-cross
build-go-turbo-cross:
	goreleaser release --rm-dist -f cross-release.yml

.PHONY: build-lib-turbo-cross
build-lib-turbo-cross:
	goreleaser release --rm-dist -f cross-lib.yml

.PHONY: stage-release
stage-release: cmd/turbo/version.go
	echo "Version: $(TURBO_VERSION)"
	echo "Tag: $(TURBO_TAG)"
	cat $(CLI_DIR)/../version.txt
	git diff -- $(CLI_DIR)/../version.txt
	git status
	@test main = "`git rev-parse --abbrev-ref HEAD`" || (echo "Refusing to publish from non-main branch `git rev-parse --abbrev-ref HEAD`" && false)
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)

	# Stop if versions are not updated.
	@test "" != "`git diff -- $(CLI_DIR)/../version.txt`" || (echo "Refusing to publish with unupdated version.txt" && false)
	@test "" != "`git diff -- $(CLI_DIR)/cmd/turbo/version.go`" || (echo "Refusing to publish with unupdated version.go" && false)

	# Prepare the packages.
	cd $(CLI_DIR)/../packages/turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/create-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-codemod && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-ignore && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-workspaces && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/eslint-plugin-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/eslint-config-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version

	git checkout -b staging-$(TURBO_VERSION)
	git commit -anm "publish $(TURBO_VERSION) to registry"
	git tag "v$(TURBO_VERSION)"
	git push origin staging-$(TURBO_VERSION) --tags --force

.PHONY: publish-turbo
publish-turbo: clean build
	echo "Version: $(TURBO_VERSION)"
	echo "Tag: $(TURBO_TAG)"

	# Include the patch in the log.
	git format-patch HEAD~1 --stdout | cat

	npm config set --location=project "//registry.npmjs.org/:_authToken" $(NPM_TOKEN)

	# Publishes the native npm modules.
	goreleaser release --rm-dist -f combined-shim.yml $(SKIP_PUBLISH)

	# Split packing from the publish step so that npm locates the correct .npmrc file.
	cd $(CLI_DIR)/../packages/turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/create-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-codemod && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-ignore && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-workspaces && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/eslint-plugin-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/eslint-config-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../

ifneq ($(SKIP_PUBLISH),--skip-publish)
	# Publish the remaining JS packages in order to avoid race conditions.
	cd $(CLI_DIR)/../
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../create-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-codemod-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-ignore-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-workspaces-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../eslint-plugin-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../eslint-config-turbo-$(TURBO_VERSION).tgz
endif

demo/lage: install
	node $(CLI_DIR)/scripts/generate.mjs lage

demo/lerna: install
	node $(CLI_DIR)/scripts/generate.mjs lerna

demo/nx: install
	node $(CLI_DIR)/scripts/generate.mjs nx

demo/turbo: install
	node $(CLI_DIR)/scripts/generate.mjs turbo

demo: demo/lage demo/lerna demo/nx demo/turbo

bench/lerna: demo/lerna
	cd $(CLI_DIR)/demo/lerna && node_modules/.bin/lerna run build

bench/lage: demo/lage
	cd $(CLI_DIR)/demo/lage && node_modules/.bin/lage build

bench/nx: demo/nx
	cd $(CLI_DIR)/demo/nx && node_modules/.bin/nx run-many --target=build --all

bench/turbo: demo/turbo turbo
	cd $(CLI_DIR)/demo/turbo && $(CLI_DIR)/turbo run test

bench: bench/lerna bench/lage bench/nx bench/turbo

clean: clean-go clean-build clean-demo clean-rust

clean-rust:
	cargo clean

clean-build:
	rm -f turbo

clean-demo:
	rm -rf node_modules
	rm -rf demo

# use target fixture-<some directory under integration_tests/_fixtures> to set up the testbed directory
.PHONY=fixture-%
fixture-%:
	$(eval $@_FIXTURE := $(@:fixture-%=%))
	@echo "fixture setup $($@_FIXTURE)"
	rm -rf testbed
	mkdir -p testbed
	../turborepo-tests/integration/tests/_helpers/setup_monorepo.sh ./testbed $($@_FIXTURE)

