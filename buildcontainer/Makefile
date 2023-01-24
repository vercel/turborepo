include .env

REGISTRY           ?= ghcr.io
TAG_VERSION        ?= snapshot
OWNER              ?= goreleaser
GIT_REPO           ?= https://github.com/vercel/turbo

ifeq ($(REGISTRY),)
	IMAGE_BASE_NAME  := $(OWNER)/goreleaser-cross-base:$(TAG_VERSION)
	IMAGE_NAME       := $(OWNER)/goreleaser-cross:$(TAG_VERSION)
	TURBO_IMAGE_NAME := $(OWNER)/turbo-cross:$(TAG_VERSION)
else
	IMAGE_BASE_NAME  := $(REGISTRY)/$(OWNER)/goreleaser-cross-base:$(TAG_VERSION)
	IMAGE_NAME       := $(REGISTRY)/$(OWNER)/goreleaser-cross:$(TAG_VERSION)
	TURBO_IMAGE_NAME := $(REGISTRY)/$(OWNER)/turbo-cross:$(TAG_VERSION)
endif

DEBIAN_FRONTEND    := noninteractive
GORELEASER_VERSION ?= 1.1.0
TINI_VERSION       ?= v0.19.0
COSIGN_VERSION     ?= 1.3.0
COSIGN_SHA256      ?= 65de2f3f2844815ed20ab939319e3dad4238a9aaaf4893b22ec5702e9bc33755

DOCKER_BUILD=docker build

SUBIMAGES = arm64 \
 amd64

.PHONY: gen-changelog
gen-changelog:
	@echo "generating changelog to changelog"
	./scripts/genchangelog.sh $(shell git describe --tags --abbrev=0) changelog.md

.PHONY: goreleaser-cross-base-%
goreleaser-cross-base-%:
	@echo "building $(IMAGE_BASE_NAME)-$(@:goreleaser-cross-base-%=%)"
	$(DOCKER_BUILD) --platform=linux/$(@:goreleaser-cross-base-%=%) -t $(IMAGE_BASE_NAME)-$(@:goreleaser-cross-base-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg OWNER=$(OWNER) \
		--build-arg GIT_REPO=$(GIT_REPO) \
		--build-arg GORELEASER_VERSION=$(GORELEASER_VERSION) \
		--build-arg TINI_VERSION=$(TINI_VERSION) \
		--build-arg COSIGN_VERSION=$(COSIGN_VERSION) \
		--build-arg COSIGN_SHA256=$(COSIGN_SHA256) \
		--build-arg DEBIAN_FRONTEND=$(DEBIAN_FRONTEND) \
		-f Dockerfile.base .

.PHONY: goreleaser-cross-%
goreleaser-cross-%:
	@echo "building $(IMAGE_NAME)-$(@:goreleaser-cross-%=%) on top of $(IMAGE_BASE_NAME)"
	$(DOCKER_BUILD) --platform=linux/$(@:goreleaser-cross-%=%) -t $(IMAGE_NAME)-$(@:goreleaser-cross-%=%) \
		--build-arg OWNER=$(OWNER) \
		--build-arg GIT_REPO=$(GIT_REPO) \
		--build-arg BASE_IMAGE=$(IMAGE_BASE_NAME) \
		--build-arg DEBIAN_FRONTEND=$(DEBIAN_FRONTEND) \
		-f Dockerfile .

.PHONY: goreleaser-cross-base
goreleaser-cross-base: $(patsubst %, goreleaser-cross-base-%,$(SUBIMAGES))

.PHONY: goreleaser-cross
goreleaser-cross: $(patsubst %, goreleaser-cross-%,$(SUBIMAGES))

.PHONY: docker-push-base-%
docker-push-base-%:
	docker push $(IMAGE_BASE_NAME)-$(@:docker-push-base-%=%)

.PHONY: docker-push-%
docker-push-%:
	docker push $(IMAGE_NAME)-$(@:docker-push-%=%)

.PHONY: docker-push-base
docker-push-base: $(patsubst %, docker-push-base-%,$(SUBIMAGES))

.PHONY: docker-push
docker-push: $(patsubst %, docker-push-%,$(SUBIMAGES))

.PHONY: manifest-create-base
manifest-create-base:
	@echo "creating base manifest $(IMAGE_BASE_NAME)"
	docker manifest rm $(IMAGE_BASE_NAME) 2> /dev/null || true
	docker manifest create $(IMAGE_BASE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_BASE_NAME)-$(arch))

.PHONY: manifest-create
manifest-create:
	@echo "creating manifest $(IMAGE_NAME)"
	docker manifest rm $(IMAGE_NAME) 2> /dev/null || true
	docker manifest create $(IMAGE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_NAME)-$(arch))

.PHONY: manifest-push-base
manifest-push-base:
	@echo "pushing base manifest $(IMAGE_BASE_NAME)"
	docker manifest push $(IMAGE_BASE_NAME)

.PHONY: manifest-push
manifest-push:
	@echo "pushing manifest $(IMAGE_NAME)"
	docker manifest push $(IMAGE_NAME)

.PHONY: turbo-cross-%
turbo-cross-%:
	# Currently, we don't layer any customizations
	@echo "tagging $(IMAGE_NAME)-$(@:turbo-cross-%=%) to $(TURBO_IMAGE_NAME)-$(@:turbo-cross-%=%)"
	docker tag $(IMAGE_NAME)-$(@:turbo-cross-%=%) $(TURBO_IMAGE_NAME)-$(@:turbo-cross-%=%)

.PHONY: turbo-cross
turbo-cross: $(patsubst %, turbo-cross-%,$(SUBIMAGES))

.PHONY: push-turbo-cross-%
push-turbo-cross-%: turbo-cross-%
	docker push $(TURBO_IMAGE_NAME)-$(@:push-turbo-cross-%=%)

.PHONY: push-turbo-cross
push-turbo-cross: $(patsubst %, push-turbo-cross-%,$(SUBIMAGES))

.PHONY: manifest-create-turbo
manifest-create-turbo:
	@echo "creating manifest $(TURBO_IMAGE_NAME)"
	docker manifest rm $(TURBO_IMAGE_NAME) 2> /dev/null || true
	docker manifest create $(TURBO_IMAGE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(TURBO_IMAGE_NAME)-$(arch))

.PHONY: manifest-push-turbo
manifest-push-turbo:
	@echo "pushing manifest $(TURBO_IMAGE_NAME)"
	docker manifest push $(TURBO_IMAGE_NAME)
