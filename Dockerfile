FROM golang:alpine as builder

RUN mkdir -p $GOPATH/src/github.com/vx-labs
WORKDIR $GOPATH/src/github.com/jbonachera/nanoproxy
COPY . ./
ENV CGO_ENABLED=0
RUN go test ./... && \
    go build -buildmode=exe -ldflags="-s -w" -a -o /bin/nanoproxy .

FROM alpine
EXPOSE 8888
ENTRYPOINT ["/bin/nanoproxy"]
CMD ["-b", ":8888"]
COPY --from=builder /bin/nanoproxy /bin/nanoproxy
