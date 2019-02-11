package requests

import (
	"io"
	"net/http"
)

func FromClientRequest(r *http.Request) (*http.Request, error) {
	ctx := r.Context()
	httpReq, err := http.NewRequest(r.Method, r.URL.String(), r.Body)
	if err != nil {
		return nil, err
	}
	for key, values := range r.Header {
		for _, value := range values {
			httpReq.Header.Add(key, value)
		}
	}
	return httpReq.WithContext(ctx), nil

}

func ProcessConnect(clientConn io.ReadWriteCloser, upstreamConn io.ReadWriteCloser) {
	readCh := make(chan struct{})
	writeCh := make(chan struct{})
	go func() {
		defer close(readCh)
		io.Copy(upstreamConn, clientConn)
	}()
	go func() {
		defer close(writeCh)
		io.Copy(clientConn, upstreamConn)
	}()
	select {
	case <-readCh:
	case <-writeCh:
	}
}
