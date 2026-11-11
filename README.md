# Kdiff4

Kdiff4 is a graphical two-way and three-way file diff and merge tool.

## Installation

Run the command for your platform to install the latest release for your user account.

**Linux x86-64 and Apple silicon macOS** (Bash or Zsh with `curl`)

```bash
installer=$(curl -fsSL https://raw.githubusercontent.com/xahon/kdiff4/master/scripts/install.sh) && bash -c "$installer"
```

Installs to `~/.local/bin` and adds it to PATH for new Bash or Zsh sessions.

**Windows x64** (PowerShell)

```powershell
& ([scriptblock]::Create((Invoke-WebRequest -UseBasicParsing https://raw.githubusercontent.com/xahon/kdiff4/master/scripts/install.ps1).Content))
```

Installs to `%LOCALAPPDATA%\Programs\Kdiff4`, updates your user PATH, and adds Kdiff4 to the Start menu.
