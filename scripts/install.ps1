& {
    $ErrorActionPreference = 'Stop'
    if ([Environment]::OSVersion.Platform -ne 'Win32NT' -or
        (Get-ItemPropertyValue -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment' -Name 'PROCESSOR_ARCHITECTURE') -ne 'AMD64') {
        throw 'Kdiff4 releases support Windows x64 with this installer.'
    }

    $binDir = Join-Path $env:LOCALAPPDATA 'Programs\Kdiff4'
    $executable = Join-Path $binDir 'kdiff4.exe'
    $download = Join-Path $binDir ("kdiff4-{0}.tmp" -f [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $binDir -Force | Out-Null

    try {
        $url = 'https://github.com/xahon/kdiff4/releases/latest/download/kdiff4-windows-x86_64.exe'
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $download
        if ([IO.File]::Exists($executable)) {
            [IO.File]::Replace($download, $executable, [NullString]::Value)
        } else {
            [IO.File]::Move($download, $executable)
        }
    } finally {
        Remove-Item -LiteralPath $download -Force -ErrorAction SilentlyContinue
    }

    $programs = [Environment]::GetFolderPath('Programs', 'DoNotVerify')
    New-Item -ItemType Directory -Path $programs -Force | Out-Null
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut((Join-Path $programs 'Kdiff4.lnk'))
    $shortcut.TargetPath = $executable
    $shortcut.WorkingDirectory = $binDir
    $shortcut.Save()

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';').TrimEnd('\') -notcontains $binDir) {
        $newPath = if ($userPath) { "$($userPath.TrimEnd(';'));$binDir" } else { $binDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }
    if (($env:Path -split ';').TrimEnd('\') -notcontains $binDir) {
        $env:Path = "$binDir;$env:Path"
    }

    Write-Output "Installed $executable and added Kdiff4 to the Start menu."
}
