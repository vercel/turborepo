# golang-cross
golang-cross

Docker container to do cross compilation (linux, windows, OSX) of go packages including support for cgo. 

## Docker

Find it on docker hub https://hub.docker.com/r/goreng/golang-cross or run 

```
docker run --rm --privileged \
  -v $PWD:/go/src/github.com/qlcchain/go-qlc \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -w /go/src/github.com/qlcchain/go-qlc \
  mailchain/goreleaser-xcgo goreleaser --snapshot --rm-dist
```
