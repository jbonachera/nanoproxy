package main

import (
	"crypto/tls"
	"encoding/base64"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/textproto"
	"net/url"
	"strings"
	"time"

	"github.com/jbonachera/nanoproxy/requests"
	"github.com/spf13/cobra"

	"github.com/spf13/viper"
)

type HTTPForwardHandler struct {
	httpClient *http.Client
	upstream   *url.URL
}

var d = &net.Dialer{
	KeepAlive: 30 * time.Second,
}

func (handler *HTTPForwardHandler) initConnectTransaction(rawConn io.ReadWriteCloser, r *http.Request) error {
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
func (handler *HTTPForwardHandler) doUpstreamConnect(w http.ResponseWriter, r *http.Request) error {
	ctx := r.Context()
	var d net.Dialer
	rawConn, err := d.DialContext(ctx, "tcp", handler.upstream.Host)
	if err != nil {
		log.Printf("textproto/dial %s failed: %v", handler.upstream.Host, err)
		return err
	}
	err = handler.initConnectTransaction(rawConn, r)
	if err != nil {
		rawConn.Close()
		log.Printf("textproto/connect failed: %v", err)
		return err
	}
	clientConn, buf, err := w.(http.Hijacker).Hijack()
	if err != nil {
		rawConn.Close()
		clientConn.Close()
		log.Printf("http hijack failed: %v", err)
		return err
	}
	rawConn.Write([]byte("HTTP/1.0 200 Connection established\n\n"))
	go func() {
		defer func() {
			rawConn.Close()
			clientConn.Close()
		}()
		requests.ProcessConnect(buf, rawConn)
	}()
	return nil
}
func (handler *HTTPForwardHandler) doConnect(w http.ResponseWriter, r *http.Request) error {
	ctx := r.Context()
	conn, err := d.DialContext(ctx, "tcp", r.Host)
	if err != nil {
		log.Printf("connect/dial %s failed: %v", r.Host, err)
		return err
	}
	clientConn, buf, err := w.(http.Hijacker).Hijack()
	if err != nil {
		conn.Close()
		clientConn.Close()
		log.Printf("http hijack failed: %v", err)
		return err
	}
	clientConn.Write([]byte("HTTP/1.0 200 Connection established\n\n"))
	go func() {
		defer func() {
			conn.Close()
			clientConn.Close()
		}()
		requests.ProcessConnect(buf, conn)
	}()
	return nil
}

func (handler *HTTPForwardHandler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	start := time.Now()
	if r.Method == http.MethodConnect {
		var err error
		if handler.upstream == nil {
			err = handler.doConnect(w, r)
		} else {
			err = handler.doUpstreamConnect(w, r)
		}
		if err == nil {
			fmt.Printf("[%v] 200 %s %s\n", time.Since(start), r.Method, r.Host)
		}
		return
	}
	httpReq, err := requests.FromClientRequest(r)
	if err != nil {
		log.Printf("ERR: failed to create proxified request: %v\n", err)
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

func NewHander(upstream string) (*HTTPForwardHandler, error) {
	proxyURL, err := url.Parse(upstream)
	if err != nil {
		return nil, err
	}
	if upstream == "" {
		proxyURL = nil
	}
	client := &http.Client{
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
		Transport: &http.Transport{
			Proxy: http.ProxyURL(proxyURL),
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
	return &HTTPForwardHandler{
		httpClient: client,
		upstream:   proxyURL,
	}, nil
}

func main() {
	config := viper.New()
	config.SetEnvPrefix("NANOPROXY")
	config.SetEnvKeyReplacer(strings.NewReplacer("-", "_"))
	config.AutomaticEnv()

	root := cobra.Command{
		Use: "nanoproxy",
		PreRun: func(cmd *cobra.Command, args []string) {
			config.BindEnv()
		},
		Run: func(cmd *cobra.Command, _ []string) {
			addr, err := net.ResolveTCPAddr("tcp", config.GetString("bind"))
			if err != nil {
				log.Fatal(err)
			}
			listener, err := net.ListenTCP("tcp", addr)
			if err != nil {
				log.Fatal(err)
			}
			srv := &http.Server{
				ReadHeaderTimeout: 5 * time.Second,
				Addr:              addr.String(),
				// Disable HTTP/2.
				TLSNextProto: make(map[string]func(*http.Server, *tls.Conn, http.Handler)),
			}

			handler, err := NewHander(config.GetString("upstream"))
			if err != nil {
				log.Fatal(err)
			}
			srv.Handler = handler

			log.Printf("proxy listening on %s", addr.String())
			log.Fatal(srv.Serve(listener))
		},
	}
	root.Flags().StringP("bind", "b", "0.0.0.0:8888", "bind to this address")
	root.Flags().StringP("upstream", "u", "", "forward requests to this proxy server")
	config.BindPFlag("bind", root.Flags().Lookup("bind"))
	config.BindPFlag("upstream", root.Flags().Lookup("upstream"))
	config.AutomaticEnv()
	err := root.Execute()
	if err != nil {
		log.Fatal(err)
	}
}
