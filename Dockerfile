FROM quay.io/vxlabs/dep as builder

RUN mkdir -p $GOPATH/src/github.com/vx-labs
WORKDIR $GOPATH/src/github.com/jbonachera/nanoproxy
COPY Gopkg* ./
RUN dep ensure -vendor-only
COPY . ./
RUN go test ./... && \
    go build -buildmode=exe -ldflags="-s -w" -a -o /bin/nanoproxy ./cmd/nanoproxy

FROM alpine
EXPOSE 8888
ENTRYPOINT ["/bin/nanoproxy"]
CMD ["-b", ":8888"]
RUN apk -U add ca-certificates && \
    rm -rf /var/cache/apk/*
COPY --from=builder /bin/nanoproxy /bin/nanoproxy
