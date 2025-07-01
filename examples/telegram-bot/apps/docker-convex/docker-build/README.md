# Building docker images

The contents of this directory are used to build the docker images for the
self-hosted backend and dashboard. If you're looking for ways to run self-hosted
Convex, see the [these instructions](../README.md). You may build the images
locally from here, but we recommend using the images we provide on GHCR.

Build the backend from scratch by running:

```sh
docker build -t convex-backend -f self-hosted/docker-build/Dockerfile.backend .
```

Build the dashboard from scratch by running:

```sh
docker build -t convex-dashboard -f self-hosted/docker-build/Dockerfile.dashboard .
```
