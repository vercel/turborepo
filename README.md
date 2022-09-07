# goreleaser-cross

Docker container to turn CGO cross-compilation pain into a pleasure. It tested on [variety of platforms](#supported-toolchains/platforms).
[Custom sysroots](#Sysroot) also can be used.

**Tip!**
Should you wish to see working [examples](#examples) instead of reading

## Credits

This project is rather cookbook combing various projects into one. Special to [osxcross](https://github.com/tpoechtrager/osxcross) for amazing cross-compile environment for OSX.

## Docker

Docker images are available on both [Github](https://ghcr.io/goreleaser/goreleaser-cross) and [Docker hub](https://hub.docker.com/r/goreleaser/goreleaser-cross).

Images from version v1.17.4 are multi-arch. Supported host are listed in the table below

| Host                 | Supported |
|----------------------|:---------:|
|  amd64               |     ✅     |
|  arm64 (aka aarch64) |     ✅     |

To run build with CGO each entry requires some environment variables

| Env variable           | value                                          |            required            | Notes                                                                                              |
|------------------------|------------------------------------------------|:------------------------------:|----------------------------------------------------------------------------------------------------|
| CGO_ENABLED            | 1                                              |              Yes               | instead of specifying it in each build it can be set globally during docker run `-e CGO_ENABLED=1` |
| CC                     | [see targets](#supported-toolchains/platforms) |            Optional            |
| CXX                    | [see targets](#supported-toolchains/platforms) |            Optional            |
| PKG_CONFIG_SYSROOT_DIR |                                                | Required if sysroot is present |
| PKG_CONFIG_PATH        |                                                |            Optional            | List of directories containing pkg-config files                                                    |

- **PKG_CONFIG_SYSROOT_DIR** modifies `-I`  and `-L` to use the directories located in target's sysroot.
- The value of `PKG_CONFIG_SYSROOT_DIR` is prefixed to `-I` and `-L`. For instance `-I/usr/include/libfoo` becomes `-I/var/target/usr/include/libfoo`
with a `PKG_CONFIG_SYSROOT_DIR` set to `/var/target` (same rule apply to `-L`)
- **PKG_CONFIG_PATH** - A colon-separated list of directories to search for `.pc` files.

## Supported toolchains/platforms

| Platform    | Arch            | CC                      | CXX                     |       Verified        |
|-------------|-----------------|-------------------------|-------------------------|:---------------------:|
| Darwin      | amd64           | o64-clang               | o64-clang++             |           ✅          |
| Darwin (M1) | arm64           | oa64-clang              | oa64-clang++            |           ✅          |
| Linux       | amd64           | gcc                     | g++                     |           ✅          |
| Linux       | arm64           | aarch64-linux-gnu-gcc   | aarch64-linux-gnu-g++   |           ✅          |
| Linux       | armhf (GOARM=5) | arm-linux-gnueabihf-gcc | arm-linux-gnueabihf-g++ | Verification required |
| Linux       | armhf (GOARM=6) | arm-linux-gnueabihf-gcc | arm-linux-gnueabihf-g++ | Verification required |
| Linux       | armhf (GOARM=7) | arm-linux-gnueabihf-gcc | arm-linux-gnueabihf-g++ |           ✅          |
| Windows     | amd64           | x86_64-w64-mingw32-gcc  | x86_64-w64-mingw32-g++  |           ✅          |
| Windows     | arm64           | /llvm-mingw/llvm-mingw-20220906-ucrt-ubuntu-18.04-aarch64/bin/aarch64-w64-mingw32-gcc | /llvm-mingw/llvm-mingw-20220906-ucrt-ubuntu-18.04-aarch64/bin/aarch64-w64-mingw32-g++ |          ✅           |

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

## Sysroot howto

Most reasonable way to make a sysroot seem to be rsync and [the example](https://github.com/goreleaser/goreleaser-cross-example) is using it. You may want to
use [the script](https://github.com/goreleaser/goreleaser-cross/blob/master/scripts/sysroot-rsync.sh) to create sysroot for your desired setup. Lets consider creating sysroot for Raspberry Pi 4
running Debian Buster.

- install all required dev packages. for this example we will install libftdi1-dev, libusb-1.0-0-dev and opencv4
  ```bash
  ./sysroot-rsync.sh pi@<ipaddress> <local destination>
  ``` 

### sshfs
Though `sshfs` is a good way to test sysroot before running rsync it introduces cons. Some packages are creating absolute links and thus pointing to wrong files when mounted (
or appear as broken). For example RPI4 running Debian Buster the library `/usr/lib/x86_x64-gnu-linux/libpthread.so` is symlink to `/lib/x86_x64-gnu-linux/libpthread.so` instead
of `../../../lib/x86_x64-gnu-linux/libpthread.so`.

## Contributing

Any contribution helping to make this project is welcome

## Examples

- [Example described in this tutorial](https://github.com/goreleaser/goreleaser-cross-example)

## Projects using

- [Akash](https://github.com/ovrclk/akash)
