# Builds dist/out.js in an image and exports it (`docker build --target artifacts -o build/artifacts .`).
FROM node:20-alpine AS build
WORKDIR /src
COPY package.json ./
COPY src ./src
COPY assets ./assets
RUN node src/build.js

FROM scratch AS artifacts
COPY --from=build /src/dist/ /
