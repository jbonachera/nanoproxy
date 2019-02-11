# Nanoproxy

Very minimalist HTTP proxy, supporting proxying to another upstream HTTP proxy, with basic auth.

## Run


```
docker run -p 8888:8888 quay.io/jbonachera/nanoproxy:v1.0.0
```

### With an upstream HTTP Proxy
```
docker run -p 8888:8888 quay.io/jbonachera/nanoproxy:v1.0.0 -u http://user:password@proxy.example.net:8123
```
