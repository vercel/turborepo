TURBO_VERSION = $(shell cat ../version.txt | sed -n '1 p')
TURBO_TAG = $(shell cat ../version.txt | sed -n '2 p')

CLI_DIR = $(shell pwd)

.PHONY: prepare-stage-release
prepare-stage-release:
	echo "Version: $(TURBO_VERSION)"
	echo "Tag: $(TURBO_TAG)"
	cat $(CLI_DIR)/../version.txt
	git diff -- $(CLI_DIR)/../version.txt
	git status
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)

	# Stop if versions are not updated.
	@test "" != "`git diff -- $(CLI_DIR)/../version.txt`" || (echo "Refusing to publish with unupdated version.txt" && false)

	# Fail if git tag already exists (this version was already successfully released)
	@if git ls-remote --tags origin "refs/tags/v$(TURBO_VERSION)" | grep -q .; then \
		echo "Error: Tag v$(TURBO_VERSION) already exists. This version was already released." && \
		exit 1; \
	fi

	# Fail if staging branch already exists (a release is in progress or a previous release failed)
	@if git ls-remote --exit-code --heads origin "staging-$(TURBO_VERSION)" >/dev/null 2>&1; then \
		echo "Error: Staging branch staging-$(TURBO_VERSION) already exists." && \
		echo "" && \
		echo "This means either:" && \
		echo "  1. A release is currently in progress (check the Actions tab)" && \
		echo "  2. A previous release failed and left the staging branch behind" && \
		echo "" && \
		echo "If a previous release failed, re-run the workflow with 'clear-staging-branch' enabled." && \
		exit 1; \
	fi

	# Prepare the packages.
	cd $(CLI_DIR)/../packages/turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/create-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/turbo-codemod && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/turbo-ignore && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/turbo-workspaces && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/turbo-gen && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/eslint-plugin-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/eslint-config-turbo && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version
	cd $(CLI_DIR)/../packages/turbo-types && pnpm version "$(TURBO_VERSION)" --allow-same-version --no-git-tag-version

	# Update turborepo skill version (in YAML frontmatter metadata block only)
	node -e "const fs=require('fs'),p='$(CLI_DIR)/../skills/turborepo/SKILL.md',c=fs.readFileSync(p,'utf-8');fs.writeFileSync(p,c.replace(/^(---\n[\s\S]*?metadata:\n\s*version:\s*).+?(\n[\s\S]*?---)/,'\$$1$(TURBO_VERSION)\$$2'))"

	# Update $schema URLs in turborepo skill files to use versioned subdomain
	node -e "\
	  const fs=require('fs'),path=require('path');\
	  const sub='v'+'$(TURBO_VERSION)'.replace(/[\.\+]/g,'-');\
	  const url='https://'+sub+'.turborepo.dev/schema.json';\
	  const re=/https:\/\/(?:v[\w-]+\.)?turborepo\.(?:dev|com)\/schema(?:\.v2)?\.json|https:\/\/turbo\.build\/schema(?:\.v2)?\.json/g;\
	  function walk(dir){for(const e of fs.readdirSync(dir,{withFileTypes:true})){const p=path.join(dir,e.name);if(e.isDirectory())walk(p);else if(e.name.endsWith('.md')){const c=fs.readFileSync(p,'utf-8');if(re.test(c)){re.lastIndex=0;fs.writeFileSync(p,c.replace(re,url))}}}}\
	  walk('$(CLI_DIR)/../skills/turborepo');\
	"

	git checkout -b staging-$(TURBO_VERSION)

.PHONY: commit-stage-release
commit-stage-release:
	node $(CLI_DIR)/../scripts/create-github-api-commit.mjs \
		--branch staging-$(TURBO_VERSION) \
		--message "publish $(TURBO_VERSION) to registry" \
		--all-tracked

.PHONY: stage-release
stage-release: prepare-stage-release
	node $(CLI_DIR)/../scripts/create-github-api-commit.mjs \
		--branch staging-$(TURBO_VERSION) \
		--message "publish $(TURBO_VERSION) to registry" \
		--all-tracked

.PHONY: create-release-tag
create-release-tag:
	@echo "Creating release tag v$(TURBO_VERSION)..."
	@REMOTE_TAG_SHA=$$(git ls-remote --tags origin "refs/tags/v$(TURBO_VERSION)" | awk '{print $$1}'); \
	LOCAL_SHA=$$(git rev-parse HEAD); \
	if [ -n "$$REMOTE_TAG_SHA" ]; then \
		if [ "$$REMOTE_TAG_SHA" = "$$LOCAL_SHA" ]; then \
			echo "Tag v$(TURBO_VERSION) already exists on remote at the correct commit. Skipping."; \
		else \
			echo "Warning: Tag v$(TURBO_VERSION) exists on remote but points to $$REMOTE_TAG_SHA, expected $$LOCAL_SHA." && \
			echo "Replacing incorrect tag..." && \
			git push origin ":refs/tags/v$(TURBO_VERSION)" && \
			git tag -f "v$(TURBO_VERSION)" && \
			git push origin "v$(TURBO_VERSION)" && \
			echo "Release tag v$(TURBO_VERSION) replaced and pushed."; \
		fi; \
	else \
		git tag "v$(TURBO_VERSION)" && \
		git push origin "v$(TURBO_VERSION)" && \
		echo "Release tag v$(TURBO_VERSION) created and pushed."; \
	fi
