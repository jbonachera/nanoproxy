package httpproxy

import (
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"time"

	"github.com/jbonachera/nanoproxy/requests"
)

type HTTPForwardHander struct {
	httpClient *http.Client
}

var d = &net.Dialer{
	KeepAlive: 30 * time.Second,
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
func (handler *HTTPForwardHander) DoConnect(w http.ResponseWriter, r *http.Request) error {
	ctx := r.Context()
	conn, err := d.DialContext(ctx, "tcp", r.Host)
	if err != nil {
		log.Printf("connect/dial %s failed: %v", r.Host, err)
		return err
	}
	w.WriteHeader(200)
	clientConn, buf, err := w.(http.Hijacker).Hijack()
	if err != nil {
		conn.Close()
		clientConn.Close()
		log.Printf("http hijack failed: %v", err)
		return err
	}
	go func() {
		defer func() {
			conn.Close()
			clientConn.Close()
		}()
		requests.ProcessConnect(buf, conn)
	}()
	return nil
}
func (handler *HTTPForwardHander) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	start := time.Now()

	log.Printf("%s %s", r.Method, r.Host)
	if r.Method == http.MethodConnect {
		err := handler.DoConnect(w, r)
		if err == nil {
			fmt.Printf("[%v] 200 %s %s\n", time.Since(start), r.Method, r.Host)
		}
	}
	httpReq, err := requests.FromClientRequest(r)
	if err != nil {
		log.Println(err)
		return
	}
	resp, err := handler.httpClient.Do(httpReq)
	if err != nil {
		log.Println(err)
		return
	}
	fmt.Printf("[%v] %d %s %s\n", time.Since(start), resp.StatusCode, r.Method, r.Host)
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
