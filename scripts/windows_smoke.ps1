param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Artifacts,
    [switch]$InspectAccessibility,
    [string]$Inspector,
    [ValidateSet('classic','siege','monokai','tokyo_night','dracula','one_dark_pro','nord','gruvbox_dark','synthwave_84','github_light','solarized_light','catppuccin_latte')][string]$ThemePalette = 'classic',
    [ValidateRange(0.75,2.0)][double]$FontScale = 1.0,
    [switch]$Light,
    [switch]$HighContrast,
    [switch]$Review,
    [switch]$CleanTracked
)
$ErrorActionPreference = 'Stop'
if ($CleanTracked -and (-not $Review -or -not $Inspector)) {
    throw 'CleanTracked acceptance requires Review and Inspector.'
}
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
$environmentNames = @('GIT_CONFIG_GLOBAL', 'GIT_CONFIG_NOSYSTEM', 'GIT_TERMINAL_PROMPT', 'APPDATA', 'EGUI_INSPECTION', 'EGUI_INSPECTION_ADDR')
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$smokeProcess = $null
try {
    $env:GIT_CONFIG_GLOBAL = "$smokeRoot\absent-global"
    $env:GIT_CONFIG_NOSYSTEM = '1'
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:APPDATA = "$smokeRoot\appdata"
    $profile = Join-Path $env:APPDATA 'recorde'
    New-Item -ItemType Directory -Path $profile | Out-Null
    $dark = if ($Light) { 'false' } else { 'true' }
    $contrast = if ($HighContrast) { 'true' } else { 'false' }
    $scale = $FontScale.ToString('0.0#', [Globalization.CultureInfo]::InvariantCulture)
    $fixtureConfig = "dark_theme = $dark`ntheme_palette = '$ThemePalette'`nfont_scale = $scale`nhigh_contrast = $contrast`n"
    [IO.File]::WriteAllText((Join-Path $profile 'config.toml'), $fixtureConfig, (New-Object Text.UTF8Encoding($false)))
    $env:EGUI_INSPECTION = $null
    $env:EGUI_INSPECTION_ADDR = $null
    if ($Inspector) {
        $inspectionClient = (Resolve-Path -LiteralPath $Inspector).Path
        $probe = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 0)
        $probe.Start()
        $inspectionAddress = "127.0.0.1:$($probe.LocalEndpoint.Port)"
        $probe.Stop()
        $env:EGUI_INSPECTION = $inspectionAddress
    }
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
    # More than one row is required for native graph keyboard-navigation checks.
    Set-Content -LiteralPath "$repo\second.txt" -Value 'second'
    git -C $repo add -- second.txt
    git -C $repo -c commit.gpgsign=false commit -m 'second fixture' >> "$smokeRoot\fixture.log"
    if ($LASTEXITCODE -ne 0) { throw 'Second fixture commit failed.' }
    if ($CleanTracked) {
        # Keep a file available to the editor without blocking reviewed history
        # operations, which correctly refuse dirty tracked files.
        Set-Content -LiteralPath "$repo\file.txt" -Value 'untracked editor fixture'
        # A second isolated tab and a workspace excluding the primary repository
        # let native acceptance cancel navigation while a primary draft is dirty.
        $workspaceRepo = Join-Path $smokeRoot 'workspace-repo'
        New-Item -ItemType Directory -Path $workspaceRepo | Out-Null
        git -C $workspaceRepo init -b main >> "$smokeRoot\fixture.log"
        if ($LASTEXITCODE -ne 0) { throw 'Workspace fixture initialization failed.' }
        git -C $workspaceRepo config user.name 'Recorde Smoke'
        git -C $workspaceRepo config user.email 'smoke@example.invalid'
        Set-Content -LiteralPath "$workspaceRepo\workspace.txt" -Value 'isolated workspace fixture'
        git -C $workspaceRepo add -- workspace.txt
        git -C $workspaceRepo -c commit.gpgsign=false commit -m 'workspace fixture' >> "$smokeRoot\fixture.log"
        if ($LASTEXITCODE -ne 0) { throw 'Workspace fixture commit failed.' }
        # Rust canonicalizes Windows worktree paths to their extended form.
        # Match those keys so restoring the primary tab cannot create a duplicate.
        $primaryConfigPath = if ($repo.StartsWith('\\?\')) { $repo } elseif ($repo.StartsWith('\\')) { '\\?\UNC\' + $repo.Substring(2) } else { '\\?\' + $repo }
        $workspaceConfigPath = if ($workspaceRepo.StartsWith('\\?\')) { $workspaceRepo } elseif ($workspaceRepo.StartsWith('\\')) { '\\?\UNC\' + $workspaceRepo.Substring(2) } else { '\\?\' + $workspaceRepo }
        $workspaceName = 'Recorde native workspace ' + (Split-Path -Leaf $smokeRoot)
        $primaryToml = ConvertTo-Json -InputObject $primaryConfigPath -Compress
        $workspaceToml = ConvertTo-Json -InputObject $workspaceConfigPath -Compress
        $workspaceNameToml = ConvertTo-Json -InputObject $workspaceName -Compress
        $fixtureConfig += "active_repo = $primaryToml`nopen_repos = [$primaryToml, $workspaceToml]`n[workspaces]`n$workspaceNameToml = [$workspaceToml]`n"
        [IO.File]::WriteAllText((Join-Path $profile 'config.toml'), $fixtureConfig, (New-Object Text.UTF8Encoding($false)))
        $navigationFixture = @{ workspace_name=$workspaceName; primary_repository=$primaryConfigPath; secondary_repository=$workspaceConfigPath } | ConvertTo-Json
        [IO.File]::WriteAllText((Join-Path $smokeRoot 'navigation-fixture.json'), $navigationFixture, (New-Object Text.UTF8Encoding($false)))
    } else {
        Set-Content -LiteralPath "$repo\hello.txt" -Value 'edited'
    }
    $smokeProcess = Start-Process -FilePath $executable -ArgumentList ('"' + $repo + '"') -WindowStyle Hidden -PassThru -RedirectStandardOutput "$smokeRoot\stdout.log" -RedirectStandardError "$smokeRoot\stderr.log"
    Start-Sleep -Seconds 8
    $smokeProcess.Refresh()
    $result = [PSCustomObject]@{ Exited = $smokeProcess.HasExited; Window = $smokeProcess.MainWindowHandle; Title = $smokeProcess.MainWindowTitle }
    $result | ConvertTo-Json | Set-Content -LiteralPath "$smokeRoot\result.json"
    if ($smokeProcess.HasExited -or $smokeProcess.MainWindowHandle -eq 0 -or $smokeProcess.MainWindowTitle -cne 'Recorde') {
        throw "Native startup failed or displayed an error dialog; see $smokeRoot"
    }
    $savedProfile = Get-Content -LiteralPath "$smokeRoot\appdata\recorde\config.toml" -Raw
    if (-not ($savedProfile.Contains($repo) -or $savedProfile.Contains($repo.Replace('\', '\\')))) {
        throw 'Application did not use the isolated profile directory.'
    }
    if ($Inspector) {
        $info = New-Object Diagnostics.ProcessStartInfo
        $info.FileName = $inspectionClient
        $info.Arguments = '"' + $inspectionAddress + '" "' + $smokeRoot + '" --graph' + $(if ($Review) { ' --review' } else { '' })
        if ($CleanTracked) { $info.Arguments += ' --clean-tracked' }
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $client = [Diagnostics.Process]::Start($info)
        @{ Purpose='Native UI inspection client'; ProcessId=$client.Id; ParentProcessId=$PID; Executable=$inspectionClient; Address=$inspectionAddress } |
            ConvertTo-Json | Set-Content -LiteralPath "$smokeRoot\inspector-ownership.json"
        $out = $client.StandardOutput.ReadToEndAsync()
        $err = $client.StandardError.ReadToEndAsync()
        $client.WaitForExit()
        [IO.File]::WriteAllText("$smokeRoot\inspector.stdout.log", $out.GetAwaiter().GetResult())
        [IO.File]::WriteAllText("$smokeRoot\inspector.stderr.log", $err.GetAwaiter().GetResult())
        if ($client.ExitCode -ne 0) { throw "Native UI inspector failed; see $smokeRoot" }
    }
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
        $expectedCurrentBranch = if ($CleanTracked) { 'Local branch develop, current branch' } else { 'Local branch main, current branch' }
        if ($null -eq $commit -or !($rows.Name -contains $expectedCurrentBranch) -or !($rows.Name -match 'Fetch')) {
            throw "Windows UI Automation is missing commit, branch or action labels; see $smokeRoot"
        }
        $window.SetFocus()
        $commit.SetFocus()
        $focusDeadline = [DateTime]::UtcNow.AddSeconds(3)
        do {
            Start-Sleep -Milliseconds 100
            $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
            if ($focused.Current.ProcessId -eq $smokeProcess.Id -and $focused.Current.Name -like 'Commit * by *') { break }
        } while ([DateTime]::UtcNow -lt $focusDeadline)
        @{ RequestedName=$commit.Current.Name; RequestedFocusable=$commit.Current.IsKeyboardFocusable; ActualName=$focused.Current.Name; ActualProcess=$focused.Current.ProcessId; ExpectedProcess=$smokeProcess.Id } |
            ConvertTo-Json | Set-Content -LiteralPath "$smokeRoot\native-uia-focus.json"
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
