param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Artifacts
)
$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $Binary).Path
$smokeRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
$repo = Join-Path $smokeRoot 'repo'
New-Item -ItemType Directory -Path $repo, "$smokeRoot\appdata", "$smokeRoot\localappdata" | Out-Null
$environmentNames = @('GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM', 'GIT_TERMINAL_PROMPT', 'APPDATA', 'LOCALAPPDATA')
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$smokeProcess = $null
try {
    $env:GIT_CONFIG_GLOBAL = "$smokeRoot\absent-global"
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:APPDATA = "$smokeRoot\appdata"
    $env:LOCALAPPDATA = "$smokeRoot\localappdata"
    git -C $repo init -b main *> "$smokeRoot\fixture.log"
    if ($LASTEXITCODE -ne 0) { throw 'Fixture initialization failed.' }
    git -C $repo config user.name 'Recorde Smoke'
    git -C $repo config user.email 'smoke@example.invalid'
    Set-Content -LiteralPath "$repo\hello.txt" -Value 'initial'
    git -C $repo add -- hello.txt
    git -C $repo -c commit.gpgsign=false commit -m fixture >> "$smokeRoot\fixture.log"
    if ($LASTEXITCODE -ne 0) { throw 'Fixture commit failed.' }
    Set-Content -LiteralPath "$repo\hello.txt" -Value 'edited'
    $smokeProcess = Start-Process -FilePath $executable -ArgumentList ('"' + $repo + '"') -WindowStyle Hidden -PassThru -RedirectStandardOutput "$smokeRoot\stdout.log" -RedirectStandardError "$smokeRoot\stderr.log"
    Start-Sleep -Seconds 8
    $smokeProcess.Refresh()
    $result = [PSCustomObject]@{ Exited = $smokeProcess.HasExited; Window = $smokeProcess.MainWindowHandle; Title = $smokeProcess.MainWindowTitle }
    $result | ConvertTo-Json | Set-Content -LiteralPath "$smokeRoot\result.json"
    if ($smokeProcess.HasExited -or $smokeProcess.MainWindowHandle -eq 0) { throw "Native startup failed; see $smokeRoot" }
    Write-Output "PASS: native window created; artifacts: $smokeRoot"
} finally {
    if ($null -ne $smokeProcess -and -not $smokeProcess.HasExited) { Stop-Process -Id $smokeProcess.Id }
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process') }
}
