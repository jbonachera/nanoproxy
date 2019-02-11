package httpproxy

import (
	"io"
	"log"
	"net"
	"net/http"
	"time"
)

type HTTPForwardHander struct {
	httpClient *http.Client
}

func NewHander() (*HTTPForwardHander, error) {
	client := &http.Client{
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
		Transport: &http.Transport{
			DialContext: (&net.Dialer{
				KeepAlive: 30 * time.Second,
			}).DialContext,
			MaxIdleConns:          100,
			IdleConnTimeout:       90 * time.Second,
			TLSHandshakeTimeout:   10 * time.Second,
			ExpectContinueTimeout: 1 * time.Second,
			ResponseHeaderTimeout: 120 * time.Second,
		},
	}
	return &HTTPForwardHander{
		httpClient: client,
	}, nil
}
func (handler *HTTPForwardHander) DoConnect(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	var d net.Dialer
	conn, err := d.DialContext(ctx, "tcp", r.Host)
	if err != nil {
		log.Printf("connect/dial failed: %v", err)
		return
	}
	defer conn.Close()
	w.WriteHeader(200)
	clientConn, _, err := w.(http.Hijacker).Hijack()
	defer clientConn.Close()
	if err != nil {
		log.Printf("http hijack failed: %v", err)
		return
	}

	readCh := make(chan struct{})
	writeCh := make(chan struct{})
	go func() {
		defer close(readCh)
		io.Copy(conn, clientConn)
	}()
	go func() {
		defer close(writeCh)
		io.Copy(clientConn, conn)
	}()
	select {
	case <-readCh:
	case <-writeCh:
	}
}
func (handler *HTTPForwardHander) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	log.Printf("%s %s", r.Method, r.Host)
	if r.Method == http.MethodConnect {
		handler.DoConnect(w, r)
		return
	}
	ctx := r.Context()
	httpReq, err := http.NewRequest(r.Method, r.URL.String(), r.Body)
	if err != nil {
		log.Println(err)
		return
	}
	httpReq = httpReq.WithContext(ctx)
	for key, values := range r.Header {
		for _, value := range values {
			httpReq.Header.Add(key, value)
		}
	}
	resp, err := handler.httpClient.Do(httpReq)
	if err != nil {
		log.Println(err)
		return
	}
	if resp.Body != nil {
		defer resp.Body.Close()
	}
	for key, values := range resp.Header {
		for _, value := range values {
			w.Header().Add(key, value)
		}
	}
	w.WriteHeader(resp.StatusCode)
	_, err = io.Copy(w, resp.Body)
	if err != nil {
		log.Printf("WARN: failed to copy response body to client: %v", err)
	}
}
