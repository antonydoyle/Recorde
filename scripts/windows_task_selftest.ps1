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
