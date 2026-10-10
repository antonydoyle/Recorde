# Destructive installer acceptance is restricted to disposable hosted CI runners.
param(
    [Parameter(Mandatory = $true)][string]$PreviousMsi,
    [Parameter(Mandatory = $true)][string]$PreviousChecksums,
    [Parameter(Mandatory = $true)][string]$CandidateMsi,
    [Parameter(Mandatory = $true)][string]$ExpectedBinary,
    [Parameter(Mandatory = $true)][string]$Artifacts
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Installer acceptance only runs on disposable GitHub-hosted runners.'
}
$previous = (Resolve-Path -LiteralPath $PreviousMsi).Path
$candidate = (Resolve-Path -LiteralPath $CandidateMsi).Path
$expected = (Resolve-Path -LiteralPath $ExpectedBinary).Path
$root = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$installed = Join-Path $env:ProgramFiles 'Recorde\recorde.exe'
if (Test-Path -LiteralPath $installed) { throw 'Refusing to alter an existing Recorde installation.' }
$installer = New-Object -ComObject WindowsInstaller.Installer
function Read-MsiProperty([string]$file, [string]$property) {
    $database = $installer.OpenDatabase($file, 0)
    $view = $database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$property'")
    $view.Execute()
    $record = $view.Fetch()
    if ($null -eq $record) { throw "Missing MSI property $property" }
    $value = $record.StringData(1)
    $view.Close()
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($record)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($view)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)
    return $value
}
function Invoke-Msi([string]$action, [string]$file, [string]$stage, [bool]$expectFailure = $false) {
    $arguments = @($action, ('"' + $file + '"'), '/qn', '/norestart', '/L*v', ('"' + "$root\$stage.log" + '"'))
    $process = Start-Process -FilePath "$env:SystemRoot\System32\msiexec.exe" -ArgumentList $arguments -WindowStyle Hidden -PassThru
    [pscustomobject]@{purpose=$stage; pid=$process.Id; parent=$PID; executable=$process.StartInfo.FileName; ports=@()} |
        ConvertTo-Json -Compress | Add-Content -LiteralPath "$root\installer-processes.jsonl"
    $process.WaitForExit()
    $code = $process.ExitCode
    if ($expectFailure) {
        if ($code -ne 1603) { throw "Downgrade must fail with 1603; got $code" }
    } elseif ($code -notin @(0, 3010)) { throw "Installer $stage failed ($code); see $root" }
}
$oldVersion = Read-MsiProperty $previous 'ProductVersion'
$newVersion = Read-MsiProperty $candidate 'ProductVersion'
$oldProduct = Read-MsiProperty $previous 'ProductCode'
$newProduct = Read-MsiProperty $candidate 'ProductCode'
if ([version]$newVersion -le [version]$oldVersion) { throw 'Candidate must be newer than baseline.' }
if ((Read-MsiProperty $previous 'UpgradeCode') -ne (Read-MsiProperty $candidate 'UpgradeCode')) { throw 'UpgradeCode changed.' }
$previousName = [IO.Path]::GetFileName($previous)
$checksum = @(Get-Content -LiteralPath $PreviousChecksums | Where-Object { $_ -match ('^[a-fA-F0-9]{64}  ' + [regex]::Escape($previousName) + '$') })
if ($checksum.Count -ne 1 -or (Get-FileHash -LiteralPath $previous -Algorithm SHA256).Hash -ne $checksum[0].Substring(0,64)) { throw 'Baseline MSI checksum mismatch.' }
$userdata = Join-Path $env:APPDATA 'recorde'
if (Test-Path -LiteralPath $userdata) { throw 'Unexpected pre-existing Recorde user data.' }
New-Item -ItemType Directory -Path "$userdata\drafts" | Out-Null
$sentinels = @{
    "$userdata\config.toml" = "dark_theme = false`nfavorites = []`n"
    "$userdata\drafts\installer-test.txt" = 'Preserve my unfinished commit message.'
    "$userdata\credentials.json" = '{"fixture":"metadata only; no credentials"}'
}
foreach ($entry in $sentinels.GetEnumerator()) { [IO.File]::WriteAllText($entry.Key,$entry.Value) }
function Assert-UserData {
    foreach ($entry in $sentinels.GetEnumerator()) {
        if (-not (Test-Path -LiteralPath $entry.Key) -or [IO.File]::ReadAllText($entry.Key) -cne $entry.Value) { throw "User data changed: $($entry.Key)" }
    }
}
$oldInstalled = $false
$newInstalled = $false
try {
    Invoke-Msi '/i' $previous 'install-baseline'
    $oldInstalled = $true
    if (-not (Test-Path -LiteralPath $installed)) { throw 'Baseline binary missing.' }
    Assert-UserData
    Invoke-Msi '/i' $candidate 'upgrade'
    $newInstalled = $true
    $oldInstalled = $false
    if ((Get-FileHash -LiteralPath $installed).Hash -ne (Get-FileHash -LiteralPath $expected).Hash) { throw 'Installed executable differs from the candidate build.' }
    if ((Get-Content (Join-Path (Split-Path $installed) 'recorde-install.json') -Raw | ConvertFrom-Json).kind -ne 'msi') { throw 'Installed update marker missing.' }
    $probe = Start-Process -FilePath $installed -ArgumentList '--version' -WindowStyle Hidden -PassThru -RedirectStandardOutput "$root\version.txt" -RedirectStandardError "$root\version-error.txt"
    [pscustomobject]@{purpose='installed version probe'; pid=$probe.Id; parent=$PID; ports=@()} | ConvertTo-Json -Compress | Add-Content "$root\installer-processes.jsonl"
    $probe.WaitForExit()
    if ($probe.ExitCode -ne 0 -or (Get-Content "$root\version.txt" -Raw).Trim() -ne "Recorde $newVersion") { throw 'Installed binary version probe failed.' }
    Assert-UserData
    Invoke-Msi '/i' $previous 'reject-downgrade' $true
    if ((Get-FileHash -LiteralPath $installed).Hash -ne (Get-FileHash -LiteralPath $expected).Hash) { throw 'Downgrade altered the candidate.' }
    Invoke-Msi '/x' $newProduct 'uninstall'
    $newInstalled = $false
    if (Test-Path -LiteralPath $installed) { throw 'Uninstall left executable behind.' }
    Assert-UserData
    [pscustomobject]@{baseline=$oldVersion; candidate=$newVersion; upgrade='passed'; downgrade='rejected'; uninstall='passed'; userdata='preserved'} |
        ConvertTo-Json | Set-Content "$root\result.json"
    Write-Output "PASS: MSI install, upgrade, downgrade guard, uninstall and user-data retention. $root"
} finally {
    if ($newInstalled) { Invoke-Msi '/x' $newProduct 'cleanup-candidate' }
    elseif ($oldInstalled) { Invoke-Msi '/x' $oldProduct 'cleanup-baseline' }
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}
