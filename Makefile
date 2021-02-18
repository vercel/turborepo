CROSS_IMAGE_NAME   := troian/golang-cross-builder
IMAGE_NAME         := troian/golang-cross
GO_VERSION         ?= 1.16.0
TAG_VERSION        := v$(GO_VERSION)
GORELEASER_VERSION := 0.156.2
GORELEASER_SHA     := 875371b95da87218e4f06f084f5c5da72fde87013790132b41b1243f33e65745
OSX_SDK            := MacOSX10.15.sdk
OSX_SDK_SUM        := 4051d210bf232ccb5eee863d6a4052afa800001810a2a42e354c9637ec78cd2c
OSX_VERSION_MIN    := 10.12
OSX_CROSS_COMMIT   := 035cc170338b7b252e3f13b0e3ccbf4411bffc41
DEBIAN_FRONTEND    := noninteractive

SUBIMAGES = linux-amd64

PUSHIMAGES = base \
	$(SUBIMAGES)

subimages: $(patsubst %, golang-cross-%,$(SUBIMAGES))

.PHONY: golang-cross-base
golang-cross-base:
	@echo "building $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)"
	docker build -t $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		--build-arg GORELEASER_VERSION=$(GORELEASER_VERSION) \
		--build-arg GORELEASER_SHA=$(GORELEASER_SHA) \
		-f Dockerfile.$(@:golang-cross-%=%) .

.PHONY: golang-cross-%
golang-cross-%: golang-cross-base
	@echo "building $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%)"
	docker build -t $(IMAGE_NAME):$(TAG_VERSION)-$(@:golang-cross-%=%) \
		--build-arg GO_VERSION=$(GO_VERSION) \
		-f Dockerfile.$(@:golang-cross-%=%) .

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

.PHONY: docker-push-%
docker-push-%:
	docker push $(IMAGE_NAME):$(TAG_VERSION)-$(@:docker-push-%=%)

.PHONY: docker-push
docker-push: $(patsubst %, docker-push-%,$(PUSHIMAGES))
	docker push $(IMAGE_NAME):$(TAG_VERSION)
