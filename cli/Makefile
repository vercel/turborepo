TURBO_VERSION = $(shell cat ../version.txt | sed -n '1 p')
TURBO_TAG = $(shell cat ../version.txt | sed -n '2 p')

CLI_DIR = $(shell pwd)

# This only builds JS packages
build:
	cd $(CLI_DIR)/../ && turbo build \
		--filter=create-turbo \
		--filter=@turbo/codemod \
		--filter=turbo-ignore \
		--filter=@turbo/workspaces \
		--filter=@turbo/gen \
		--filter=eslint-plugin-turbo \
		--filter=eslint-config-turbo \
		--filter=@turbo/types

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
publish-turbo: build
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

# use target fixture-<some directory under turborepo-tests/integration/fixtures> to set up the testbed directory
.PHONY=fixture-%
fixture-%:
	$(eval $@_FIXTURE := $(@:fixture-%=%))
	@echo "fixture setup $($@_FIXTURE)"
	rm -rf testbed
	mkdir -p testbed
	../turborepo-tests/helpers/setup_integration_test.sh ./testbed $($@_FIXTURE)

