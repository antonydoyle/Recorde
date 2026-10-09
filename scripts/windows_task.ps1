param(
    [string]$Executable,
    [string[]]$Arguments = @(),
    [string]$Purpose,
    [string]$WorkingDirectory = (Get-Location).Path,
    [string]$Artifacts,
    [ValidateRange(1, 1800)][int]$TimeoutSeconds = 120,
    [ValidateRange(128, 16384)][int]$MaxMemoryMiB = 4096,
    [switch]$Child,
    [string]$Spec
)
$ErrorActionPreference = 'Stop'

if ($Child) {
    # The parent assigns this waiting process to its private job before releasing it.
    if ([Console]::ReadLine() -ne 'GO') { exit 125 }
    $taskSpec = Get-Content -LiteralPath $Spec -Raw | ConvertFrom-Json
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $taskSpec.Executable
    $info.Arguments = $taskSpec.CommandArguments
    $info.WorkingDirectory = $taskSpec.WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $worker = [Diagnostics.Process]::Start($info)
    @{ Pid = $worker.Id; Purpose = $taskSpec.Purpose; Ports = @(); StartedUtc = [DateTime]::UtcNow.ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath ($Spec + '.worker.json')
    $stdout = [IO.File]::Open($taskSpec.Stdout, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    $stderr = [IO.File]::Open($taskSpec.Stderr, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
    try {
        $copyOut = $worker.StandardOutput.BaseStream.CopyToAsync($stdout)
        $copyErr = $worker.StandardError.BaseStream.CopyToAsync($stderr)
        $worker.WaitForExit()
        # An orphaned helper may inherit an output pipe after the actual worker
        # exits. Bound draining so the parent can terminate the owned job instead
        # of waiting for that helper indefinitely.
        if ([Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($copyOut, $copyErr), 2000)) {
            $null = $copyOut.GetAwaiter().GetResult()
            $null = $copyErr.GetAwaiter().GetResult()
        } else {
            $worker.StandardOutput.Close()
            $worker.StandardError.Close()
        }
        exit $worker.ExitCode
    } finally { $stdout.Dispose(); $stderr.Dispose() }
}

if (!$Executable -or !$Purpose -or !$Artifacts) { throw 'Executable, Purpose and Artifacts are required.' }
$Executable = (Get-Command $Executable -CommandType Application -ErrorAction Stop).Source
$WorkingDirectory = (Resolve-Path -LiteralPath $WorkingDirectory).Path
$runRoot = Join-Path ([IO.Path]::GetFullPath($Artifacts)) ([Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $runRoot | Out-Null

if (-not ('RecordeTaskJob' -as [type])) {
Add-Type @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public sealed class RecordeTaskJob : IDisposable {
    [StructLayout(LayoutKind.Sequential)] struct Basic {
        public long ProcessTime, JobTime; public uint Flags;
        public UIntPtr MinWorkingSet, MaxWorkingSet; public uint ActiveProcesses;
        public UIntPtr Affinity; public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct Io { public ulong A,B,C,D,E,F; }
    [StructLayout(LayoutKind.Sequential)] struct Limits {
        public Basic Basic; public Io Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job, int kind, ref Limits limits, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool QueryInformationJobObject(IntPtr job, int kind, IntPtr data, uint size, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    IntPtr handle;
    public RecordeTaskJob(ulong memoryBytes) {
        handle = CreateJobObject(IntPtr.Zero, null);
        if (handle == IntPtr.Zero) throw new Win32Exception();
        var limits = new Limits();
        limits.Basic.Flags = 0x2000 | 0x8 | 0x200; // Kill on close; process and memory limits.
        limits.Basic.ActiveProcesses = 32;
        limits.JobMemory = (UIntPtr)memoryBytes;
        if (!SetInformationJobObject(handle, 9, ref limits, (uint)Marshal.SizeOf(limits))) {
            CloseHandle(handle); handle = IntPtr.Zero; throw new Win32Exception();
        }
    }
    public void Assign(IntPtr process) { if (!AssignProcessToJobObject(handle, process)) throw new Win32Exception(); }
    public int[] Members() {
        IntPtr data = Marshal.AllocHGlobal(65536);
        try {
            if (!QueryInformationJobObject(handle, 3, data, 65536, IntPtr.Zero)) throw new Win32Exception();
            int count = Marshal.ReadInt32(data, 4);
            if (count < 0 || count > 1024) throw new InvalidOperationException("Unexpected job size");
            var ids = new int[count];
            for (int i=0; i<count; i++) ids[i] = Marshal.ReadIntPtr(data, 8+i*IntPtr.Size).ToInt32();
            return ids;
        } finally { Marshal.FreeHGlobal(data); }
    }
    public void Stop() { if (handle != IntPtr.Zero && !TerminateJobObject(handle, 125)) throw new Win32Exception(); }
    public void Dispose() { if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; } }
    public static string Quote(string value) {
        var text = new StringBuilder("\""); int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            text.Append('\\', c == '"' ? slashes * 2 + 1 : slashes); text.Append(c); slashes = 0;
        }
        return text.Append('\\', slashes * 2).Append('"').ToString();
    }
    public delegate bool WindowCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(WindowCallback callback, IntPtr data);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    public static long[] GitWindows() {
        var result = new List<long>();
        EnumWindows((window, data) => {
            var title = new StringBuilder(1024); GetWindowText(window, title, title.Capacity);
            foreach (var program in new[]{"git.exe", "ssh-add.exe", "ssh-keygen.exe", "cargo.exe", "rustc.exe"}) {
                if (title.ToString().IndexOf(program, StringComparison.OrdinalIgnoreCase) >= 0) { result.Add(window.ToInt64()); break; }
            }
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }
}
'@
}

function Get-TaskPorts([int[]]$Ids) {
    if ($Ids.Count -eq 0) { return @() }
    @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { $_.State -eq 'Listen' -and $_.OwningProcess -in $Ids } |
        ForEach-Object { [PSCustomObject]@{ Protocol='TCP'; Pid=$_.OwningProcess; Address=$_.LocalAddress; Port=$_.LocalPort } }) +
    @(Get-NetUDPEndpoint -ErrorAction Stop | Where-Object { $_.OwningProcess -in $Ids } |
        ForEach-Object { [PSCustomObject]@{ Protocol='UDP'; Pid=$_.OwningProcess; Address=$_.LocalAddress; Port=$_.LocalPort } })
}

$specPath = Join-Path $runRoot 'task.json'
@{ Executable=$Executable; CommandArguments=(($Arguments | ForEach-Object { [RecordeTaskJob]::Quote($_) }) -join ' ');
    Purpose=$Purpose; WorkingDirectory=$WorkingDirectory; Stdout=(Join-Path $runRoot 'stdout.log'); Stderr=(Join-Path $runRoot 'stderr.log') } |
    ConvertTo-Json | Set-Content -LiteralPath $specPath
$baselineWindows = @([RecordeTaskJob]::GitWindows())
$job = New-Object RecordeTaskJob -ArgumentList ([uint64]$MaxMemoryMiB * 1MB)
$wrapper = $null
$assigned = $false
$failure = $null
$exitCode = 125
$members = @()
$ports = @()
try {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = (Get-Process -Id $PID).Path
    $info.Arguments = '-NoProfile -NonInteractive -File ' + [RecordeTaskJob]::Quote($PSCommandPath) + ' -Child -Spec ' + [RecordeTaskJob]::Quote($specPath)
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardInput = $true
    $wrapper = [Diagnostics.Process]::Start($info)
    # Retained process handle and a private kernel job establish ownership, not a name match.
    $job.Assign($wrapper.Handle)
    $assigned = $true
    @{ Purpose=$Purpose; WrapperPid=$wrapper.Id; ParentPid=$PID; Executable=$Executable; Ports=@(); StartedUtc=[DateTime]::UtcNow.ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runRoot 'ownership.json')
    Write-Output "Started '$Purpose'; wrapper PID $($wrapper.Id); no ports requested; ledger: $runRoot"
    $wrapper.StandardInput.WriteLine('GO')
    $wrapper.StandardInput.Close()
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (!$wrapper.WaitForExit(500)) {
        if ($timer.Elapsed.TotalSeconds -gt $TimeoutSeconds) { throw "Task exceeded ${TimeoutSeconds}s; stopping its job." }
        if (@([RecordeTaskJob]::GitWindows() | Where-Object { $_ -notin $baselineWindows }).Count -gt 0) {
            throw 'Unexpected Git/tool console window detected. Stopping this task; shared terminal hosts will not be killed automatically.'
        }
        foreach ($file in @('stdout.log','stderr.log')) {
            $log = Get-Item -LiteralPath (Join-Path $runRoot $file) -ErrorAction SilentlyContinue
            if ($null -ne $log -and $log.Length -gt 20MB) { throw 'Task output exceeded 20 MiB.' }
        }
    }
    $exitCode = $wrapper.ExitCode
    foreach ($file in @('stdout.log','stderr.log')) {
        $log = Get-Item -LiteralPath (Join-Path $runRoot $file) -ErrorAction SilentlyContinue
        if ($null -ne $log -and $log.Length -gt 20MB) { throw 'Task output exceeded 20 MiB.' }
    }
} catch { $failure = $_.Exception.Message }
finally {
    try {
        if ($assigned) {
            $ids = @($job.Members())
            $members = @(Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -in $ids } |
                Select-Object ProcessId,ParentProcessId,CreationDate,ExecutablePath,CommandLine)
            # Verify and record command line, parent and listening ports before terminating.
            $ports = @(Get-TaskPorts $ids)
            @{ Purpose=$Purpose; Processes=$members; Ports=$ports } | ConvertTo-Json -Depth 5 |
                Set-Content -LiteralPath (Join-Path $runRoot 'before-cleanup.json')
            $job.Stop()
        } elseif ($null -ne $wrapper) {
            # It has not received GO and cannot have launched the requested command.
            $wrapper.StandardInput.Close()
            if (!$wrapper.WaitForExit(3000)) { $wrapper.Kill() }
        }
    } finally { $job.Dispose() }
    if ($null -ne $wrapper) { $null = $wrapper.WaitForExit(3000) }
    # Job termination is asynchronous. In particular, compiler helpers can still
    # appear in CIM briefly after TerminateJobObject has returned. Wait for the
    # recorded identities to exit; never terminate a reused PID or unrelated host.
    $cleanupTimer = [Diagnostics.Stopwatch]::StartNew()
    do {
        $survivors = @(Get-CimInstance Win32_Process | Where-Object {
            $candidate = $_
            @($members | Where-Object { $_.ProcessId -eq $candidate.ProcessId -and $_.CreationDate -eq $candidate.CreationDate }).Count -gt 0
        })
        if ($survivors.Count -eq 0 -or $cleanupTimer.Elapsed.TotalSeconds -ge 10) { break }
        Start-Sleep -Milliseconds 200
    } while ($true)
    $survivingIds = @($survivors | ForEach-Object { $_.ProcessId })
    $remainingPorts = @(Get-TaskPorts $survivingIds)
    $windows = @([RecordeTaskJob]::GitWindows() | Where-Object { $_ -notin $baselineWindows })
    if ($survivors.Count -gt 0 -or $remainingPorts.Count -gt 0 -or $windows.Count -gt 0) { $failure = 'Cleanup verification failed; inspect result.json. No unrelated process was terminated.' }
    @{ Purpose=$Purpose; ExitCode=$exitCode; Failure=$failure; RemainingPids=$survivingIds; RemainingPorts=$remainingPorts;
        UnattributedConsoleWindows=$windows; FinishedUtc=[DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $runRoot 'result.json')
    Write-Output "Cleanup: $($survivors.Count) task processes, $($remainingPorts.Count) task listeners, $($windows.Count) new tool windows remain. Artifacts: $runRoot"
}
if ($failure) { throw $failure }
if ($exitCode -ne 0) { throw "Task exited with code $exitCode; see $runRoot" }
