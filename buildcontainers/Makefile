
BASE_IMAGE ?= ghcr.io/gsoltis/goreleaser-cross:v1.18.5

SUBIMAGES = arm64 \
 amd64

.PHONY: turbo-cross-%
turbo-cross-%:
	@echo "building for $(@:turbo-cross-%=%)"
	docker build \
		--build-arg BASE_IMAGE=$(BASE_IMAGE) \
		-t ghcr.io/gsoltis/turbo-cross:v1.18.5-$(@:turbo-cross-%=%) \
		-f Dockerfile .

.PHONY: turbo-cross
turbo-cross: $(patsubst %, turbo-cross-%,$(SUBIMAGES))

push-turbo-ccross: turbo-cross


