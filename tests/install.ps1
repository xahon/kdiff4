$ErrorActionPreference = 'Stop'
$installer = Join-Path $PSScriptRoot '../scripts/install.ps1'
$originalLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
$binDir = Join-Path $env:LOCALAPPDATA 'Programs\Kdiff4'
$executable = Join-Path $binDir 'kdiff4.exe'

function Invoke-WebRequest($Uri, $OutFile, [switch]$UseBasicParsing) {
    if ($Uri -ne 'https://github.com/xahon/kdiff4/releases/latest/download/kdiff4-windows-x86_64.exe') {
        throw "Unexpected download URL $Uri"
    }
    Set-Content -LiteralPath $OutFile -Value 'new version'
    if ($failDownload) { throw 'Download interrupted' }
}

function New-Item($ItemType, $Path, [switch]$Force) {
    if ($Path -eq $binDir) {
        Microsoft.PowerShell.Management\New-Item -ItemType $ItemType -Path $Path -Force:$Force
    } elseif ($Path -ne [Environment]::GetFolderPath('Programs', 'DoNotVerify')) {
        throw "Unexpected directory $Path"
    }
}

function New-Object($ComObject) {
    if ($ComObject -ne 'WScript.Shell') { throw "Unexpected COM object $ComObject" }
    $shell = [pscustomobject]@{}
    $shell | Add-Member ScriptMethod CreateShortcut {
        param($Path)
        if ((Split-Path -Leaf $Path) -ne 'Kdiff4.lnk') { throw "Unexpected shortcut $Path" }
        $shortcut = [pscustomobject]@{ TargetPath = ''; WorkingDirectory = '' }
        $shortcut | Add-Member ScriptMethod Save {
            if ($this.TargetPath -ne $executable -or $this.WorkingDirectory -ne $binDir) {
                throw 'Incorrect shortcut target'
            }
            throw 'Shortcut verified'
        }
        $shortcut
    }
    $shell
}

try {
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    Set-Content -LiteralPath $executable -Value 'old version'
    $failDownload = $true
    try {
        & $installer
        throw 'Expected a failed download'
    } catch {
        if ($_.Exception.Message -ne 'Download interrupted') { throw }
    }
    if ((Get-Content -LiteralPath $executable) -ne 'old version') { throw 'Previous install was modified' }
    if (Get-ChildItem -LiteralPath $binDir -Filter '*.tmp') { throw 'Temporary download was left behind' }

    $failDownload = $false
    try {
        & $installer
        throw 'Expected the shortcut to be verified'
    } catch {
        if ($_.Exception.Message -notlike '*Shortcut verified*') { throw }
    }
    if ((Get-Content -LiteralPath $executable) -ne 'new version') { throw 'Executable was not replaced' }
} finally {
    Get-ChildItem -LiteralPath $binDir -File -ErrorAction SilentlyContinue | Remove-Item -Force
    Remove-Item -LiteralPath $binDir, (Split-Path -Parent $binDir), $env:LOCALAPPDATA -Force -ErrorAction SilentlyContinue
    $env:LOCALAPPDATA = $originalLocalAppData
}
