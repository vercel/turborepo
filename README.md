# golang-cross [![Actions Status](https://github.com/troian/golang-cross/workflows/Docker%20Image%20CI/badge.svg)](https://github.com/troian/golang-cross/actions)

Docker container to turn CGO cross-compilation pain into a pleasure on [variety of platforms](#supported-toolchains/platforms) including usage of custom sysroots. 
Although cross-compilation without CGO works well too,
it is probably better to call goreleaser directly as it saves time on downloading quite big Docker image, especially on CI environment

{{box op="start" cssClass="boxed tipBox"}}
**Tip!**
Should you wish to see working [examples](#examples) instead of reading
{{box op="end"}}

## Credits
This project is rather cookbook. Actual work to create cross-compile environment is done by [osxcross](https://github.com/tpoechtrager/osxcross) and [golang-cross](https://github.com/gythialy/golang-cross)

To run build with CGO each entry requires some environment variables

Env variable | value | required | Notes
---|---|:---:|---
CGO_ENABLED|1|Yes|instead of specifying it in each build it can be set globally during docker run `-e CGO_ENABLED=1`
CC| [see targets](#supported-toolchains/platforms) | Optional |
CXX| [see targets](#supported-toolchains/platforms)| Optional |
PKG_CONFIG_SYSROOT_DIR| | Required if sysroot is present |
PKG_CONFIG_PATH| | Optional | List of directories containing pkg-config files

**PKG_CONFIG_SYSROOT_DIR** Modifies `-I`  and `-L` to use the directories located in target sys root.
This option is required when cross-compiling packages that use pkg-config to determine CFLAGS and LDFLAGS. 
`-I` and `-L` are modified to point to the new system root.
This means that a `-I/usr/include/libfoo` will become `-I/var/target/usr/include/libfoo`
with a `PKG_CONFIG_SYSROOT_DIR` equal to `/var/target` (same rule apply to `-L`)

**PKG_CONFIG_PATH** - A colon-separated list of directories to search for `.pc` files.
The default directory will always be searched after searching the path;
the default is `libdir/pkgconfig:datadir/pkgconfig` where `libdir` is the libdir
for pkg-config and `datadir` is the datadir for pkg-config when it was installed.

## Supported toolchains/platforms
Platform | Arch | CC | CXX | Verified
---|---|---|---|:---:|
Darwin|amd64|o64-clang|o64-clang++|✅
Linux|amd64|gcc|g++|✅
Linux|arm64|aarch64-linux-gnu-gcc|aarch64-linux-gnu-g++|✅
Linux|armhf (GOARM=5)|arm-linux-gnueabihf-gcc|arm-linux-gnueabihf-g++|Verification required
Linux|armhf (GOARM=6)|arm-linux-gnueabihf-gcc|arm-linux-gnueabihf-g++|Verification required
Linux|armhf (GOARM=7)|arm-linux-gnueabihf-gcc|arm-linux-gnueabihf-g++|✅
Window|amd64|x86_64-w64-mingw32-gcc|x86_64-w64-mingw32-g++|Verification required

## Docker
### Environment variables
- GPG_KEY - defaults to /secrets/key.gpg. ignored if file not found
- DOCKER_USERNAME
- DOCKER_PASSWORD
- DOCKER_HOST - defaults to `hub.docker.io`. ignored if `DOCKER_USERNAME` and `DOCKER_PASSWORD` are empty or `DOCKER_CREDS_FILE` is present
- DOCKER_CREDS_FILE - path to file with docker login credentials in colon separated format `user:password:<registry>`. useful when push to multiple docker registries required
    ```
    user1:password1:hub.docker.io
    user2:password2:registry.gitlab.com
    ```
- DOCKER_FAIL_ON_LOGIN_ERROR - fail on docker login error
- GITHUB_TOKEN - github auth token to deploy release

## How to run
See Makefile for examples 

## Sysroot
TODO

## Contributing
Any contribution helping to make this project is welcome

## Examples
 - [Example described in this tutorial](https://github.com/troian/golang-cross-example)

## Projects using
 - [Akash](https://github.com/ovrclk/akash)
