package requests

import (
	"io"
	"log"
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

func ProcessConnect(clientConn io.ReadWriter, upstreamConn io.ReadWriter) {
	readCh := make(chan struct{})
	writeCh := make(chan struct{})
	go func() {
		defer close(readCh)
		_, err := io.Copy(upstreamConn, clientConn)
		if err != nil {
			log.Printf("CONNECT: read failed: %v", err)
		}
	}()
	go func() {
		defer close(writeCh)
		_, err := io.Copy(clientConn, upstreamConn)
		if err != nil {
			log.Printf("CONNECT: write failed: %v", err)
		}
	}()
	<-readCh
	<-writeCh
}
