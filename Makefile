PACKAGE_NAME          := github.com/troian/golang-cross
GOLANG_CROSS_VERSION  ?= v1.15.2

.PHONY: release-dry-run
release-dry-run:
	@docker run \
		--rm \
		--privileged \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-v `pwd`:/go/src/$(PACKAGE_NAME) \
		-w /go/src/$(PACKAGE_NAME) \
		troian/golang-cross:${GOLANG_CROSS_VERSION} \
		--rm-dist --skip-validate --skip-publish

.PHONY: release
release:
	@if [ ! -f ".release-env" ]; then \
		@echo "\033[91m.release-env is required for release\033[0m";\
		@exit 1;\
	fi
	@docker run \
		--rm \
		--privileged \
		--env-file .release-env \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-v `pwd`:/go/src/$(PACKAGE_NAME) \
		-w /go/src/$(PACKAGE_NAME) \
		troian/golang-cross:${GOLANG_CROSS_VERSION} \
		release --rm-dist
