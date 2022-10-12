include .env

OWNER      ?= vercel
VERSION    ?= v1.18.5
IMAGE_NAME ?= ghcr.io/$(OWNER)/turbo-cross:$(VERSION)
BASE_IMAGE ?= ghcr.io/$(OWNER)/goreleaser-cross:$(VERSION)
GIT_REPO   ?= https://github.com/$(OWNER)/turborepo


SUBIMAGES = arm64 \
 amd64

.PHONY: turbo-cross-%
turbo-cross-%:
	@echo "building for $(@:turbo-cross-%=%)"
	docker build \
		--platform=linux/$(@:turbo-cross-%=%) \
		--build-arg BASE_IMAGE=$(BASE_IMAGE) \
		--build-arg GIT_REPO=$(GIT_REPO) \
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
	# delete an existing one if it exists. Recreating with the same SHAs should result in an identical manifest
	docker manifest rm $(IMAGE_NAME) 2> /dev/null || true
	docker manifest create $(IMAGE_NAME) $(foreach arch,$(SUBIMAGES), --amend $(IMAGE_NAME)-$(arch))

.PHONY: manifest-push
manifest-push: manifest-create
	@echo "pushing manifest $(IMAGE_NAME)"
	docker manifest push $(IMAGE_NAME)

