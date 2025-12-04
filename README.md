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

The configuration file is located at `~/.config/nanoproxy/nanoproxy.toml`.

### System Configuration

```toml
[system]
max_connections = 2048
log_level = "info"
```

### Network Detection

Nanoproxy supports two methods for detecting which network you're on and loading the appropriate PAC file:

#### DNS-Based Detection (Default)

Detects the network by matching DNS resolver IPs against configured subnets:

```toml
# Optional: explicitly set detection type (defaults to "dns" if not specified)
detection_type = "dns"

[[resolvconf_rules]]
resolver_subnet = '10.241.52.0/24'
pac_url = 'http://pac.example.com:8080/proxy.pac'
when_match = 'networksetup -listallnetworkservices | while read a; do sudo networksetup -setautoproxystate "$a" off; sudo networksetup -setwebproxy "$a" 127.0.0.1 8888; sudo networksetup -setsecurewebproxy "$a" 127.0.0.1 8888; done'
when_no_match = 'networksetup -listallnetworkservices | while read a; do sudo networksetup -setautoproxystate "$a" off; sudo networksetup -setwebproxy "$a" 127.0.0.1 8888; sudo networksetup -setsecurewebproxy "$a" 127.0.0.1 8888; done'
```

#### Interface-Based Detection

Detects the network by matching the default route network interface name. Useful when:
- Your network changes between Ethernet, WiFi, and VPN
- You want to detect VPN connections by interface pattern (e.g., `utun*`)
- DNS-based detection isn't reliable in your environment

```toml
detection_type = "route"

# Home Ethernet
[[gateway_rules]]
default_route_interface = 'en0'
pac_url = 'http://home-pac.local/proxy.pac'
when_match = 'echo "Ethernet connected"'

# Corporate VPN (matches utun0, utun1, utun2, etc.)
[[gateway_rules]]
default_route_interface = 'utun*'
pac_url = 'http://corp-pac.example.com/proxy.pac'
when_match = 'echo "VPN connected"'

# WiFi fallback
[[gateway_rules]]
default_route_interface = 'en1'
pac_url = 'http://wifi-pac.local/proxy.pac'
```

**Optional IP Subnet Matching:**

You can also match based on the IP address assigned to the interface. The rule triggers only if BOTH the interface name AND the IP subnet match:

```toml
detection_type = "route"

# Home network - Ethernet with specific subnet
[[gateway_rules]]
default_route_interface = 'en0'
interface_ip_subnet = '192.168.1.0/24'
pac_url = 'http://home-pac.local/proxy.pac'

# Office network - Ethernet with different subnet
[[gateway_rules]]
default_route_interface = 'en0'
interface_ip_subnet = '10.0.0.0/8'
pac_url = 'http://office-pac.company.com/proxy.pac'

# VPN with specific corporate subnet
[[gateway_rules]]
default_route_interface = 'utun*'
interface_ip_subnet = '172.16.0.0/12'
pac_url = 'http://corp-vpn-pac.example.com/proxy.pac'
```

**Finding your interface name:**
- macOS/Linux: Run `ifconfig` or `ip addr`
- Common interfaces:
  - `en0` - Ethernet
  - `en1` - WiFi
  - `utun0`, `utun1`, etc. - VPN tunnels
  - `wlan0`, `wlan1` - WiFi (Linux)

**Wildcard patterns:**
- `*` - matches any interface
- `utun*` - matches utun0, utun1, utun2, etc.
- `en*` - matches en0, en1, en2, etc.

**Note:** Interface detection polls the default route every 5 seconds to detect network changes.

### Authentication Rules

Configure proxy authentication credentials:

```toml
[[auth_rules]]
remote_pattern = '.example.com'
username = 'xxx'
password_command = 'security find-internet-password -s "some.proxy.url.com" -w'
```