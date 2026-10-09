param([Parameter(Mandatory=$true)][string]$Artifacts)
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$listenerScript = Join-Path $testRoot 'listener.ps1'
@'
$listener = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
$listener.Start()
try {
    @{ Pid=$PID; Port=$listener.LocalEndpoint.Port } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'listener.json')
    Start-Sleep -Seconds 60
} finally { $listener.Stop() }
'@ | Set-Content -LiteralPath $listenerScript
$parentScript = Join-Path $testRoot 'parent.ps1'
@'
$info = New-Object Diagnostics.ProcessStartInfo
$info.FileName = (Get-Process -Id $PID).Path
$info.Arguments = '-NoProfile -NonInteractive -File "' + (Join-Path $PSScriptRoot 'listener.ps1') + '"'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$child = [Diagnostics.Process]::Start($info)
$child.WaitForExit()
'@ | Set-Content -LiteralPath $parentScript
$timedOut = $false
try {
    & (Join-Path $PSScriptRoot 'windows_task.ps1') -Executable (Get-Process -Id $PID).Path `
        -Arguments @('-NoProfile','-NonInteractive','-File',$parentScript) `
        -Purpose 'Self-test: timeout must stop an owned grandchild and release its TCP port' `
        -Artifacts $testRoot -TimeoutSeconds 10 -MaxMemoryMiB 512
} catch {
    if ($_.Exception.Message -notlike '*exceeded*') { throw }
    $timedOut = $true
}
if (!$timedOut) { throw 'Expected the test task to hit its timeout.' }
$listener = Get-Content -LiteralPath (Join-Path $testRoot 'listener.json') -Raw | ConvertFrom-Json
if (Get-NetTCPConnection -State Listen -LocalPort $listener.Port -ErrorAction SilentlyContinue) { throw 'Test listener survived cleanup.' }
$resultFiles = @(Get-ChildItem -Path (Join-Path $testRoot '*\result.json'))
if ($resultFiles.Count -ne 1) { throw 'Expected exactly one task result.' }
$resultFile = $resultFiles[0]
$result = Get-Content -LiteralPath $resultFile.FullName -Raw | ConvertFrom-Json
if ($result.RemainingPids.Count -or $result.RemainingPorts.Count -or $result.UnattributedConsoleWindows.Count) { throw 'Cleanup left resources behind.' }
$before = Get-Content -LiteralPath (Join-Path $resultFile.DirectoryName 'before-cleanup.json') -Raw | ConvertFrom-Json
if (@($before.Ports | Where-Object { $_.Port -eq $listener.Port -and $_.Pid -eq $listener.Pid }).Count -ne 1) {
    throw 'Listener ownership was not recorded before cleanup.'
}
Write-Output "PASS: timed-out task and grandchild stopped; port $($listener.Port) closed; no tool windows. Artifacts: $testRoot"

# A successful compiler can leave a helper (such as VCTIP) alive after its own
# exit. The job must clean that helper up too, and wait for termination to settle.
$successRoot = Join-Path $testRoot 'successful-parent'
New-Item -ItemType Directory -Path $successRoot | Out-Null
Copy-Item -LiteralPath $listenerScript -Destination (Join-Path $successRoot 'listener.ps1')
$successParent = Join-Path $successRoot 'parent.ps1'
@'
$info = New-Object Diagnostics.ProcessStartInfo
$info.FileName = (Get-Process -Id $PID).Path
$info.Arguments = '-NoProfile -NonInteractive -File "' + (Join-Path $PSScriptRoot 'listener.ps1') + '"'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$child = [Diagnostics.Process]::Start($info)
$timer = [Diagnostics.Stopwatch]::StartNew()
while (!(Test-Path -LiteralPath (Join-Path $PSScriptRoot 'listener.json'))) {
    if ($child.HasExited -or $timer.Elapsed.TotalSeconds -gt 5) { throw 'Listener failed to start' }
    Start-Sleep -Milliseconds 100
}
# Deliberately exit without waiting for the helper.
'@ | Set-Content -LiteralPath $successParent
& (Join-Path $PSScriptRoot 'windows_task.ps1') -Executable (Get-Process -Id $PID).Path `
    -Arguments @('-NoProfile','-NonInteractive','-File',$successParent) `
    -Purpose 'Self-test: successful parent must not leave an owned helper or listener' `
    -Artifacts $successRoot -TimeoutSeconds 20 -MaxMemoryMiB 512
$listener = Get-Content -LiteralPath (Join-Path $successRoot 'listener.json') -Raw | ConvertFrom-Json
if (Get-NetTCPConnection -State Listen -LocalPort $listener.Port -ErrorAction SilentlyContinue) { throw 'Successful task left its listener behind.' }
$resultFile = Get-ChildItem -Path (Join-Path $successRoot '*\result.json') | Select-Object -First 1
$result = Get-Content -LiteralPath $resultFile.FullName -Raw | ConvertFrom-Json
if ($result.ExitCode -ne 0 -or $result.RemainingPids.Count -or $result.RemainingPorts.Count -or $result.UnattributedConsoleWindows.Count) { throw 'Successful task cleanup failed.' }
Write-Output 'PASS: successful parent helper stopped; no task listeners or windows remain.'
