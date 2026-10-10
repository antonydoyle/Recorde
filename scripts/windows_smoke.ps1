param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Artifacts,
    [switch]$InspectAccessibility
)
$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $Binary).Path
$nativeUpdates = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'recorde\updates'
$updateState = Join-Path $nativeUpdates 'state.json'
if ((Test-Path -LiteralPath $updateState) -and (Get-Content -LiteralPath $updateState -Raw | ConvertFrom-Json).preferences.automatic_checks) {
    throw 'Startup smoke requires automatic update checks to be disabled; preferences were not changed.'
}
if (Test-Path -LiteralPath (Join-Path (Split-Path $executable) '.recorde-update-pending.json')) {
    throw 'Refusing startup smoke while this installation has a pending update recovery marker.'
}
$smokeRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
$repo = Join-Path $smokeRoot 'repo'
New-Item -ItemType Directory -Path $smokeRoot | Out-Null
# Artifact drives can grant write access to all authenticated users. The
# isolated application-data fixture must model a private user profile so the
# updater's ownership checks remain enabled during startup acceptance.
$owner = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetOwner($owner)
$acl.SetAccessRuleProtection($true, $false)
$rule = New-Object Security.AccessControl.FileSystemAccessRule($owner, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
$acl.AddAccessRule($rule)
Set-Acl -LiteralPath $smokeRoot -AclObject $acl
New-Item -ItemType Directory -Path $repo, "$smokeRoot\appdata" | Out-Null
$environmentNames = @('GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM', 'GIT_TERMINAL_PROMPT', 'APPDATA')
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$smokeProcess = $null
try {
    $env:GIT_CONFIG_GLOBAL = "$smokeRoot\absent-global"
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:APPDATA = "$smokeRoot\appdata"
    # Config/account metadata are isolated; OS-vault/repository/update locks
    # intentionally keep their stable native per-user locations.
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
    if ($InspectAccessibility) {
        # Run on an interactive Windows test desktop (for example the hosted
        # native acceptance job). This queries the actual UI Automation bridge,
        # independently of the development egui tree inspector.
        Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
        $window = [System.Windows.Automation.AutomationElement]::FromHandle($smokeProcess.MainWindowHandle)
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $elements = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $rows = @($elements | ForEach-Object {
                [PSCustomObject]@{ Name=$_.Current.Name; Type=$_.Current.ControlType.ProgrammaticName; Focusable=$_.Current.IsKeyboardFocusable }
            })
            $commit = @($elements | Where-Object { $_.Current.Name -like 'Commit * by *' }) | Select-Object -First 1
            if ($null -ne $commit) { break }
            Start-Sleep -Milliseconds 200
        } while ([DateTime]::UtcNow -lt $deadline)
        $rows | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$smokeRoot\native-uia.json"
        if ($null -eq $commit -or !($rows.Name -match '^Branch main') -or !($rows.Name -match 'Fetch')) {
            throw "Windows UI Automation is missing commit, branch or action labels; see $smokeRoot"
        }
        $commit.SetFocus()
        Start-Sleep -Milliseconds 250
        $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
        if ($focused.Current.ProcessId -ne $smokeProcess.Id -or $focused.Current.Name -notlike 'Commit * by *') {
            throw 'Native UI Automation focus did not reach the labelled commit row.'
        }
        Write-Output 'PASS: Windows UI Automation commit/branch/action labels and native focus'
    }
    Write-Output "PASS: native window created; artifacts: $smokeRoot"
} finally {
    if ($null -ne $smokeProcess -and -not $smokeProcess.HasExited) {
        $owned = Get-CimInstance Win32_Process -Filter "ProcessId = $($smokeProcess.Id)"
        $ports = @(Get-NetTCPConnection -OwningProcess $smokeProcess.Id -State Listen -ErrorAction SilentlyContinue | Select-Object LocalAddress, LocalPort)
        @{ ProcessId=$smokeProcess.Id; ParentProcessId=$owned.ParentProcessId; CommandLine=$owned.CommandLine; Ports=$ports } |
            ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$smokeRoot\cleanup-ownership.json"
        if ($owned.ParentProcessId -eq $PID -and $owned.ExecutablePath -eq $executable -and $owned.CommandLine.Contains($repo)) {
            try {
                Stop-Process -InputObject $smokeProcess -ErrorAction Stop
            } catch {
                # The GUI can exit between the ownership check and cleanup.
                # Only an already-exited owned process makes that race benign.
                $smokeProcess.Refresh()
                if (-not $smokeProcess.HasExited) { throw }
            }
            $smokeProcess.WaitForExit(5000) | Out-Null
        } else {
            Write-Warning "Owned smoke process identity changed; leaving PID $($smokeProcess.Id) for supervisor review."
        }
    }
    foreach ($name in $environmentNames) { [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process') }
}
