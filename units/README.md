# Startup units

## MacOS

The nanoproxy.plist file can be used to install the service as a user. Edit it first :
- ProgramArguments: change the path to the binary
- StandardOutPath: Replace USERNAME by your username for logs to be visible through Console.app
- StandardErrorPath: Same as above

Then copy it to ~/Library/LaunchAgents/nanoproxy.plist then run :

```bash
launchctl load -w ~/Library/LaunchAgents/nanoproxy.plist
```

You can unload the service with :

```bash
launchctl unload -w ~/Library/LaunchAgents/nanoproxy.plist
```