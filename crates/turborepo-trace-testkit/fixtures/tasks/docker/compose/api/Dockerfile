ARG NODE_VERSION=20
FROM node:${NODE_VERSION} AS build
ARG APP
WORKDIR /app
COPY shared ./shared
COPY ${APP}/package.json ${APP}/
COPY ${APP}/src ${APP}/src
RUN npm --prefix ${APP} ci

FROM build AS test
RUN npm --prefix api test

FROM node:${NODE_VERSION}-slim AS runtime
COPY --from=build /app /app
CMD ["node", "/app/api/src/index.js"]
