# Nanoproxy
A lightweight HTTP forward proxy that supports loading routing rules from PAC URLs.

## Setup

Download the latest release from https://github.com/jbonachera/nanoproxy/releases.
Unzip the file, then make the binary executable. On macOS, you may need to remove the Apple quarantine attribute.

```shell
curl -LO https://github.com/jbonachera/nanoproxy/releases/download/master/nanoproxy-macos-universal.zip
unzip nanoproxy-macos-universal.zip
cd nanoproxy-macos-universal/
xattr -d com.apple.quarantine ./nanoproxy
chmod +x ./nanoproxy
```

## Configuration

The configuration file is located at `~/.config/nanoproxy/nanoproxy.toml`. Here's an example :

```toml
[system]
max_connections = 2048
log_level = "info"

[[auth_rules]]
remote_pattern = '.example.com'
username = 'xxx'
password_command = 'security find-internet-password  -s "some.proxy.url.com" -w'

[[resolvconf_rules]]
resolver_subnet = '10.241.52.0/24'
pac_url = 'http://pac.example.com:8080/proxy.pac'
when_match = 'networksetup -listallnetworkservices | while read a; do sudo networksetup -setautoproxystate "$a" off; sudo networksetup -setwebproxy "$a" 127.0.0.1 8888; sudo networksetup -setsecurewebproxy "$a" 127.0.0.1 8888; done'
when_no_match = 'networksetup -listallnetworkservices | while read a; do sudo networksetup -setautoproxystate "$a" off; sudo networksetup -setwebproxy "$a" 127.0.0.1 8888; sudo networksetup -setsecurewebproxy "$a" 127.0.0.1 8888; done'
```