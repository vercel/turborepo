include .env

REGISTRY           ?= ghcr.io
TAG_VERSION        := v$(GO_VERSION)

ifeq ($(REGISTRY),)
	IMAGE_BASE_NAME := troian/golang-cross-base:$(TAG_VERSION)
	IMAGE_NAME      := troian/golang-cross:$(TAG_VERSION)
else
	IMAGE_BASE_NAME := $(REGISTRY)/troian/golang-cross-base:$(TAG_VERSION)
	IMAGE_NAME      := $(REGISTRY)/troian/golang-cross:$(TAG_VERSION)
endif

OSX_SDK            := MacOSX12.0.sdk
OSX_SDK_SUM        := ac07f28c09e6a3b09a1c01f1535ee71abe8017beaedd09181c8f08936a510ffd
OSX_VERSION_MIN    := 10.9
OSX_CROSS_COMMIT   := e59a63461da2cbc20cb0a5bbfc954730e50a5472
DEBIAN_FRONTEND    := noninteractive
GORELEASER_VERSION ?= 1.1.0
TINI_VERSION       ?= v0.19.0
GORELEASER_TAG     ?= $(shell git describe --tags --abbrev=0)
COSIGN_VERSION     ?= 1.3.0
COSIGN_SHA256      ?= 65de2f3f2844815ed20ab939319e3dad4238a9aaaf4893b22ec5702e9bc33755

DOCKER_BUILD=docker build

SUBIMAGES = arm64 \
 amd64

.PHONY: gen-changelog
gen-changelog:
	@echo "generating changelog to changelog"
	./scripts/genchangelog.sh $(shell git describe --tags --abbrev=0) changelog.md

.PHONY: golang-cross-base-%
golang-cross-base-%:
	@echo "building $(IMAGE_BASE_NAME)-$(@:golang-cross-base-%=%)"
	$(DOCKER_BUILD) --platform=linux/$(@:golang-cross-base-%=%) -t $(IMAGE_BASE_NAME)-$(@:golang-cross-base-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg GORELEASER_VERSION=$(GORELEASER_VERSION) \
		--build-arg TINI_VERSION=$(TINI_VERSION) \
		--build-arg COSIGN_VERSION=$(COSIGN_VERSION) \
		--build-arg COSIGN_SHA256=$(COSIGN_SHA256) \
		--build-arg DEBIAN_FRONTEND=$(DEBIAN_FRONTEND) \
		-f Dockerfile.base .

.PHONY: golang-cross-%
golang-cross-%:
	@echo "building $(IMAGE_NAME)-$(@:golang-cross-%=%)"
	$(DOCKER_BUILD) --platform=linux/$(@:golang-cross-%=%) -t $(IMAGE_NAME)-$(@:golang-cross-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg OSX_SDK=$(OSX_SDK) \
		--build-arg OSX_SDK_SUM=$(OSX_SDK_SUM) \
		--build-arg OSX_VERSION_MIN=$(OSX_VERSION_MIN) \
		--build-arg OSX_CROSS_COMMIT=$(OSX_CROSS_COMMIT) \
		--build-arg DEBIAN_FRONTEND=$(DEBIAN_FRONTEND) \
		-f Dockerfile .

.PHONY: golang-cross-base
golang-cross: $(patsubst %, golang-cross-base-%,$(SUBIMAGES))

.PHONY: golang-cross
golang-cross: $(patsubst %, golang-cross-%,$(SUBIMAGES))

.PHONY: docker-push-base-%
docker-push-%:
	docker push $(IMAGE_BASE_NAME)-$(@:docker-push-base-%=%)

.PHONY: docker-push-%
docker-push-%:
	docker push $(IMAGE_NAME)-$(@:docker-push-%=%)

.PHONY: docker-push-base
docker-push: $(patsubst %, docker-push-base-%,$(SUBIMAGES))

.PHONY: docker-push
docker-push: $(patsubst %, docker-push-%,$(SUBIMAGES))

.PHONY: manifest-create-base
manifest-create:
	@echo "creating base manifest $(IMAGE_BASE_NAME)"
	docker manifest create $(IMAGE_BASE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_BASE_NAME)-$(arch))

.PHONY: manifest-create
manifest-create:
	@echo "creating manifest $(IMAGE_NAME)"
	docker manifest create $(IMAGE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_NAME)-$(arch))
	@echo "creating base manifest $(IMAGE_NAME)-base"
	docker manifest create $(IMAGE_NAME)-base $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_NAME)-base-$(arch))

.PHONY: manifest-push-base
manifest-push:
	@echo "pushing base manifest $(IMAGE_BASE_NAME)"
	docker manifest push $(IMAGE_BASE_NAME)

.PHONY: manifest-push
manifest-push:
	@echo "pushing manifest $(IMAGE_NAME)"
	docker manifest push $(IMAGE_NAME)

.PHONY: tags
tags:
	@echo $(IMAGE_NAME) $(foreach arch,$(SUBIMAGES), $(IMAGE_NAME)-$(arch))

.PHONY: tag
tag:
	@echo $(TAG_VERSION)
