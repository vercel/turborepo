TURBO_VERSION = $(shell cat ../version.txt | sed -n '1 p')
TURBO_TAG = $(shell cat ../version.txt | sed -n '2 p')

EXT :=
FFI_LIB :=
ifeq ($(OS), Windows_NT)
	UNAME := Windows
	EXT = .exe
	FFI_LIB = turborepo_ffi.lib
else
	UNAME := $(shell uname -s)
	FFI_LIB = libturborepo_ffi.a
endif

# Strip debug info
GO_FLAGS += "-ldflags=-s -w"

# Avoid embedding the build path in the executable for more reproducible builds
GO_FLAGS += -trimpath

CLI_DIR = $(shell pwd)

# allow opting in to the rust codepaths
GO_TAG ?= go

GO_FILES = $(shell find . -name "*.go")
SRC_FILES = $(shell find . -name "*.go" | grep -v "_test.go")
GENERATED_FILES = internal/turbodprotocol/turbod.pb.go internal/turbodprotocol/turbod_grpc.pb.go

turbo: go-turbo$(EXT)
	cargo build --manifest-path ../crates/turborepo/Cargo.toml

go-turbo$(EXT): $(GENERATED_FILES) $(SRC_FILES) go.mod turborepo-ffi-install
	CGO_ENABLED=1 go build $(GO_FLAGS) -tags $(GO_TAG) -o go-turbo$(EXT) ./cmd/turbo

.PHONY: turborepo-ffi-install
turborepo-ffi-install: turborepo-ffi
	cp -r ../crates/turborepo-ffi/bindings.h \
	      ../crates/turborepo-ffi/target/release/$(FFI_LIB) \
	      ./internal/ffi

.PHONY: turborepo-ffi
turborepo-ffi:
	cd ../crates/turborepo-ffi && CARGO_TARGET_DIR=./target cargo build --release

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

.PHONY: clean-go
clean-go:
	go clean -testcache ./...

.PHONY: test-go
test-go: $(GENERATED_FILES) $(GO_FILES) go.mod go.sum
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

ifneq ($(SKIP_PUBLISH),--skip-publish)
	# Publish the remaining JS packages in order to avoid race conditions.
	cd $(CLI_DIR)/../
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../create-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-codemod-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-ignore-$(TURBO_VERSION).tgz
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

CRAM_ENV := .cram_env

$(CRAM_ENV)/bin/pip:
	python3 -m venv $(CRAM_ENV)

$(CRAM_ENV)/bin/prysk: $(CRAM_ENV)/bin/pip
	$(CRAM_ENV)/bin/pip install prysk

INTEGRATION_TEST_FILES = $(shell find integration_tests -name "*.t")

integration-tests: $(CRAM_ENV)/bin/prysk turbo $(INTEGRATION_TEST_FILES) corepack turbo
	$(CRAM_ENV)/bin/prysk --shell=`which bash` $(INTEGRATION_TEST_FILES)

integration-tests-interactive: $(CRAM_ENV)/bin/prysk turbo $(INTEGRATION_TEST_FILES) corepack turbo
	$(CRAM_ENV)/bin/prysk --shell=`which bash` -i $(INTEGRATION_TEST_FILES)

# use target testbed-<some directory under integration_tests> to set up the testbed directory
.PHONY=testbed-%
testbed-%:
	$(eval $@_TEST := $(@:testbed-%=%))
	@echo "testbed setup $($@_TEST)"
	rm -rf testbed
	mkdir -p testbed
	./integration_tests/$($@_TEST)/setup.sh testbed

