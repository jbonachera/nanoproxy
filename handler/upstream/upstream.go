package upstream

import (
	"encoding/base64"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/textproto"
	"net/url"
	"time"

	"github.com/jbonachera/nanoproxy/requests"
)

type UpstreamForwardHandler struct {
	httpClient *http.Client
	upstream   *url.URL
}

func NewHander(upstream string) (*UpstreamForwardHandler, error) {
	upstreamURL, err := url.Parse(upstream)
	if err != nil {
		return nil, err
	}
	client := &http.Client{
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
		Transport: &http.Transport{
			Proxy: http.ProxyURL(upstreamURL),
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
	return &UpstreamForwardHandler{
		httpClient: client,
		upstream:   upstreamURL,
	}, nil
}

func (handler *UpstreamForwardHandler) InitConnect(rawConn io.ReadWriteCloser, r *http.Request) error {
	var err error
	conn := textproto.NewConn(rawConn)
	if user := handler.upstream.User.String(); user != "" {
		auth := fmt.Sprintf("Basic %s", base64.StdEncoding.EncodeToString([]byte(handler.upstream.User.String())))
		err = conn.Writer.PrintfLine(fmt.Sprintf("CONNECT %s HTTP/1.1\nHost: %s\nProxy-Authorization: %s\n", r.Host, r.Header.Get("Host"), auth))
	} else {
		err = conn.Writer.PrintfLine(fmt.Sprintf("CONNECT %s HTTP/1.1\nHost: %s\n", r.Host, r.Header.Get("Host")))
	}
	if err != nil {
		log.Printf("textproto/write failed: %v", err)
		return err
	}
	resp, err := conn.ReadLine()
	if err != nil {
		log.Printf("textproto/write failed: %v", err)
		return err
	}
	if resp != "HTTP/1.1 200 Connection established" {
		log.Printf("proxy refused CONNECT: %s", resp)
		return err
	}
	return nil
}
func (handler *UpstreamForwardHandler) DoConnect(w http.ResponseWriter, r *http.Request) {
	ctx := r.Context()
	var d net.Dialer
	rawConn, err := d.DialContext(ctx, "tcp", handler.upstream.Host)
	if err != nil {
		log.Printf("textproto/dial failed: %v", err)
		return
	}
	defer rawConn.Close()
	err = handler.InitConnect(rawConn, r)
	if err != nil {
		log.Printf("textproto/connect failed: %v", err)
		return
	}
	w.WriteHeader(200)
	clientConn, _, err := w.(http.Hijacker).Hijack()
	defer clientConn.Close()
	if err != nil {
		log.Printf("http hijack failed: %v", err)
		return
	}
	requests.ProcessConnect(clientConn, rawConn)
}
func (handler *UpstreamForwardHandler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	log.Printf("%s %s", r.Method, r.Host)
	if r.Method == http.MethodConnect {
		handler.DoConnect(w, r)
		return
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
