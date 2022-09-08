include .env

OWNER      ?= vercel
VERSION    ?= v1.18.5
IMAGE_NAME ?= ghcr.io/$(OWNER)/turbo-cross:$(VERSION)
BASE_IMAGE ?= ghcr.io/$(OWNER)/goreleaser-cross:$(VERSION)


SUBIMAGES = arm64 \
 amd64

.PHONY: turbo-cross-%
turbo-cross-%:
	@echo "building for $(@:turbo-cross-%=%)"
	docker build \
		--build-arg BASE_IMAGE=$(BASE_IMAGE) \
		-t $(IMAGE_NAME)-$(@:turbo-cross-%=%) \
		-f Dockerfile .

.PHONY: turbo-cross
turbo-cross: $(patsubst %, turbo-cross-%,$(SUBIMAGES))

.PHONY: push-turbo-cross-%
push-turbo-cross-%: turbo-cross-%
	docker push $(IMAGE_NAME)-$(@:push-turbo-cross-%=%)

.PHONY: push-turbo-cross
push-turbo-cross: $(patsubst %, push-turbo-cross-%,$(SUBIMAGES))

.PHONY: manifest-create
manifest-create: push-turbo-cross
	@echo "creating manifest $(IMAGE_NAME)"
	docker manifest create $(IMAGE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_NAME)-$(arch))

.PHONY: manifest-push
manifest-push: manifest-create
	@echo "pushing manifest $(IMAGE_NAME)"
	docker manifest push $(IMAGE_NAME)

