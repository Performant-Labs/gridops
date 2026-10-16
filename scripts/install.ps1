# Installs the GridOps host agent on Windows, enrolls the machine and keeps the
# agent running as a scheduled task at logon (so Docker Desktop is available).
#
#   irm https://gridops.example.com/install.ps1 | iex
#
# GridOps serves this script with its own address and version filled in, so
# the agent always matches the control plane.
#
# Settings (environment variables):
#   GRIDOPS_URL               GridOps address the agent connects to
#   GRIDOPS_VERSION           agent version to install, e.g. 0.1.0 (default: latest)
#   GRIDOPS_INSTALL_DIR       where the binary goes (default: %LOCALAPPDATA%\GridOps\bin)
#   GRIDOPS_ENROLLMENT_CODE   enroll without a prompt
#   GRIDOPS_AGENT_IMAGES      Docker runner images the machine may run
#   GRIDOPS_NO_ENROLL=1       install only
#   GRIDOPS_NO_SERVICE=1      don't register or start the scheduled task
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

& {
    $repository = 'iamngoni/gridops'
    $servedUrl = '__GRIDOPS_URL__'
    $servedVersion = '__GRIDOPS_VERSION__'
    if ($servedUrl.StartsWith('__')) { $servedUrl = '' }
    if ($servedVersion.StartsWith('__')) { $servedVersion = '' }

    $url = if ($env:GRIDOPS_URL) { $env:GRIDOPS_URL } else { $servedUrl }
    $version = if ($env:GRIDOPS_VERSION) { $env:GRIDOPS_VERSION } else { $servedVersion }
    $version = $version.TrimStart('v')
    $installDir = if ($env:GRIDOPS_INSTALL_DIR) { $env:GRIDOPS_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'GridOps\bin' }
    $stateDir = if ($env:GRIDOPS_AGENT_STATE_DIR) { $env:GRIDOPS_AGENT_STATE_DIR } else { Join-Path $env:USERPROFILE '.gridops\host-agent' }
    $taskName = 'GridOps host agent'

    $arch = switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { 'x64' }
        default { throw "gridops: unsupported architecture $($env:PROCESSOR_ARCHITECTURE)" }
    }
    $asset = "gridops-host-agent-windows-$arch.exe"
    $base = if ($version) { "https://github.com/$repository/releases/download/v$version" } else { "https://github.com/$repository/releases/latest/download" }

    $work = Join-Path ([IO.Path]::GetTempPath()) ("gridops-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        Write-Host "Downloading $asset $version..."
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile (Join-Path $work $asset)
        Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $work 'SHA256SUMS')
        $expected = Get-Content (Join-Path $work 'SHA256SUMS') |
            ForEach-Object { $parts = $_ -split '\s+', 2; if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset) { $parts[0] } } |
            Select-Object -First 1
        if (-not $expected) { throw "gridops: SHA256SUMS has no entry for $asset" }
        $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $asset)).Hash.ToLowerInvariant()
        if ($actual -ne $expected.ToLowerInvariant()) { throw "gridops: checksum mismatch for $asset" }

        $existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        if ($existing) { Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue }
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        $exe = Join-Path $installDir 'gridops-host-agent.exe'
        Copy-Item -Force (Join-Path $work $asset) $exe
    } finally {
        Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
    }
    Write-Host "Installed $exe ($(& $exe --version))."

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (($userPath -split ';') -contains $installDir)) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $installDir).TrimStart(';'), 'User')
        Write-Host "Added $installDir to your PATH for new terminals."
    }
    if ($env:GRIDOPS_AGENT_IMAGES) {
        [Environment]::SetEnvironmentVariable('GRIDOPS_AGENT_IMAGES', $env:GRIDOPS_AGENT_IMAGES, 'User')
    }

    $savedConfig = Join-Path $stateDir 'agent.json'
    $enrolled = { (Test-Path $savedConfig) -and ((Get-Content -Raw $savedConfig) -match 'fleet_url') }
    $enrolledNow = $false
    if ($env:GRIDOPS_NO_ENROLL -ne '1' -and -not (& $enrolled)) {
        $enrolledNow = $true
        if ($url) { $env:GRIDOPS_FLEET_URL = $url }
        & $exe enroll --no-start
        if ($LASTEXITCODE -ne 0) { throw 'gridops: enrollment failed' }
    }

    if ($env:GRIDOPS_NO_SERVICE -eq '1' -or -not (& $enrolled)) {
        Write-Host "Start the agent with: $exe run"
        return
    }
    $running = Get-CimInstance Win32_Process -Filter "Name = 'gridops-host-agent.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -match '\brun\b' }
    if ($running -and -not $existing) {
        throw 'gridops: an agent is already running outside the scheduled task; stop it, then run this again (or set GRIDOPS_NO_SERVICE=1)'
    }
    $action = New-ScheduledTaskAction -Execute $exe -Argument 'run'
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    # Tasks default to below-normal priority; native jobs build under the
    # agent, so it runs at normal priority like an agent started by hand.
    $settings = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 4
    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $taskName
    Write-Host "The agent runs as the scheduled task '$taskName'."
    if ($enrolledNow) { Write-Host 'Approve the machine in GridOps under Machines.' }
}
