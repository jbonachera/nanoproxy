# Nanoproxy
A lightweight HTTP forward proxy that supports loading routing rules from PAC URLs.

## Setup

Download the latest release from https://github.com/jbonachera/nanoproxy/releases.
Unzip the file, then make the binary executable. On macOS, you may need to remove the Apple quarantine attribute.

```shell
curl -O https://github.com/jbonachera/nanoproxy/releases/download/master/nanoproxy-macos-universal.zip
unzip nanoproxy-macos-universal.zip
cd nanoproxy-macos-universal/
xattr -d com.apple.quarantine ./nanoproxy
chmod +x ./nanoproxy
```