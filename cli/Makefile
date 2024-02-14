TURBO_VERSION = $(shell cat ../version.txt | sed -n '1 p')
TURBO_TAG = $(shell cat ../version.txt | sed -n '2 p')

EXT :=
ifeq ($(OS),Windows_NT)
	UNAME := Windows
	EXT = .exe
else
	UNAME := $(shell uname -s)
endif

CLI_DIR = $(shell pwd)

# We do not set go-turbo as a dependency because the Rust build.rs
# script will call it for us and copy over the binary
turbo:
	cargo build -p turbo

turbo-prod:
	cargo build --release --manifest-path ../crates/turborepo/Cargo.toml

turbo-capnp:
	cd ../crates/turborepo-lib/src/hash && capnp compile -I std -ogo proto.capnp && mv ./proto.capnp.go ../../../../cli/internal/fs/hash/capnp

install: | ./package.json
	pnpm install --filter=cli

corepack:
	which corepack || npm install -g corepack@latest
	corepack enable

build: install
	cd $(CLI_DIR)/../ && pnpm build:turbo
	cd $(CLI_DIR)/../ && pnpm install --filter=create-turbo && pnpm turbo-prebuilt build --filter=create-turbo...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/codemod && pnpm turbo-prebuilt build --filter=@turbo/codemod...
	cd $(CLI_DIR)/../ && pnpm install --filter=turbo-ignore && pnpm turbo-prebuilt build --filter=turbo-ignore...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/workspaces && pnpm turbo-prebuilt build --filter=@turbo/workspaces...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/gen && pnpm turbo-prebuilt build --filter=@turbo/gen...
	cd $(CLI_DIR)/../ && pnpm install --filter=eslint-plugin-turbo && pnpm turbo-prebuilt build --filter=eslint-plugin-turbo...
	cd $(CLI_DIR)/../ && pnpm install --filter=eslint-config-turbo && pnpm turbo-prebuilt build --filter=eslint-config-turbo...
	cd $(CLI_DIR)/../ && pnpm install --filter=@turbo/types && pnpm turbo-prebuilt build --filter=@turbo/types...

.PHONY: stage-release
stage-release:
	echo "Version: $(TURBO_VERSION)"
	echo "Tag: $(TURBO_TAG)"
	cat $(CLI_DIR)/../version.txt
	git diff -- $(CLI_DIR)/../version.txt
	git status
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)

	# Stop if versions are not updated.
	@test "" != "`git diff -- $(CLI_DIR)/../version.txt`" || (echo "Refusing to publish with unupdated version.txt" && false)

	# Prepare the packages.
	cd $(CLI_DIR)/../packages/turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/create-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-codemod && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-ignore && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-workspaces && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-gen && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/eslint-plugin-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/eslint-config-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version
	cd $(CLI_DIR)/../packages/turbo-types && pnpm version "$(TURBO_VERSION)" --allow-same-version

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
	# TODO: do this without goreleaser.
	goreleaser release --rm-dist -f combined-shim.yml $(SKIP_PUBLISH)

	# Split packing from the publish step so that npm locates the correct .npmrc file.
	cd $(CLI_DIR)/../packages/turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/create-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-codemod && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-ignore && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-workspaces && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-gen && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/eslint-plugin-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/eslint-config-turbo && pnpm pack --pack-destination=$(CLI_DIR)/../
	cd $(CLI_DIR)/../packages/turbo-types && pnpm pack --pack-destination=$(CLI_DIR)/../

ifneq ($(SKIP_PUBLISH),--skip-publish)
	# Publish the remaining JS packages in order to avoid race conditions.
	cd $(CLI_DIR)/../
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../create-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-codemod-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-ignore-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-workspaces-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-gen-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../eslint-plugin-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../eslint-config-turbo-$(TURBO_VERSION).tgz
	npm publish -ddd --tag $(TURBO_TAG) $(CLI_DIR)/../turbo-types-$(TURBO_VERSION).tgz
endif

clean: clean-build clean-rust

clean-rust:
	cargo clean

clean-build:
	rm -f turbo

# use target fixture-<some directory under turborepo-tests/integration/fixtures> to set up the testbed directory
.PHONY=fixture-%
fixture-%:
	$(eval $@_FIXTURE := $(@:fixture-%=%))
	@echo "fixture setup $($@_FIXTURE)"
	rm -rf testbed
	mkdir -p testbed
	../turborepo-tests/helpers/setup_integration_test.sh ./testbed $($@_FIXTURE)

