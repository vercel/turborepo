FROM mhart/alpine-node:15.8.0

# Create app directory
WORKDIR /usr/src/app

# First copy only stuff required to install deps
# so that Docker will cache/skip this if only
# source code changes between builds
COPY package.json package.json
COPY yarn.lock yarn.lock
RUN yarn install

# Now copy the source files
COPY . ./

CMD [ "yarn", "start" ]

