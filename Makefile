IMAGE_NAME         := troian/golang-cross
GHCR_IMAGE_NAME    ?= ghcr.io/$(IMAGE_NAME)
GO_VERSION         ?= 1.17.2
TAG_VERSION        := v$(GO_VERSION)
GORELEASER_VERSION := 0.182.1
GORELEASER_SHA     := bb0b3a96bb38ba86fb3f363d303ce6079c04ada2797a892bed2e2a61ad41daf2
OSX_SDK            := MacOSX11.1.sdk
OSX_SDK_SUM        := 0a9b0bae4623960483d882fb8b7c8fca66e8863ac69d9066bafe0a3d12b67293
OSX_VERSION_MIN    := 10.13
OSX_CROSS_COMMIT   := 035cc170338b7b252e3f13b0e3ccbf4411bffc41
DEBIAN_FRONTEND    := noninteractive
TINI_VERSION       ?= v0.19.0
GORELEASER_TAG     ?= $(shell git describe --tags --abbrev=0)

SUBIMAGES = linux-amd64

PUSHIMAGES = base \
	$(SUBIMAGES)

subimages: $(patsubst %, golang-cross-%,$(SUBIMAGES))

.PHONY: gen-changelog
gen-changelog:
	@echo "generating changelog to changelog"
	./scripts/genchangelog.sh "$(GORELEASER_TAG)" changelog.md

.PHONY: golang-cross-base
golang-cross-base:
	@echo "building $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)"
	docker build -t $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg GORELEASER_VERSION=$(GORELEASER_VERSION) \
		--build-arg GORELEASER_SHA=$(GORELEASER_SHA) \
		--build-arg TINI_VERSION=$(TINI_VERSION) \
		-f Dockerfile.$(@:golang-cross-%=%) .
	docker tag $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) $(GHCR_IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)

.PHONY: golang-cross-%
golang-cross-%: golang-cross-base
	@echo "building $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)"
	docker build -t $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		-f Dockerfile.$(@:golang-cross-%=%) .
	docker tag $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) $(GHCR_IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)

.PHONY: golang-cross
golang-cross: golang-cross-base
	@echo "building $(IMAGE_NAME):$(TAG_VERSION)"
	docker build -t $(IMAGE_NAME):$(TAG_VERSION) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg OSX_SDK=$(OSX_SDK) \
		--build-arg OSX_SDK_SUM=$(OSX_SDK_SUM) \
		--build-arg OSX_VERSION_MIN=$(OSX_VERSION_MIN) \
		--build-arg OSX_CROSS_COMMIT=$(OSX_CROSS_COMMIT) \
		--build-arg DEBIAN_FRONTEND=$(DEBIAN_FRONTEND) \
		-f Dockerfile.full .
	docker tag $(IMAGE_NAME):$(TAG_VERSION) $(GHCR_IMAGE_NAME):$(TAG_VERSION)
	docker tag $(IMAGE_NAME):$(TAG_VERSION) $(GHCR_IMAGE_NAME):latest

.PHONY: docker-push-%
docker-push-%:
	docker push $(GHCR_IMAGE_NAME):$(TAG_VERSION)-$(@:docker-push-%=%)
	docker push $(GHCR_IMAGE_NAME):$(TAG_VERSION)-$(@:docker-push-%=%)

.PHONY: docker-push
docker-push: $(patsubst %, docker-push-%,$(PUSHIMAGES))
	docker push $(GHCR_IMAGE_NAME):$(TAG_VERSION)
	docker push $(GHCR_IMAGE_NAME):latest
