# ============================================================================
# One snapshot of a suspected stall, taken BEFORE anything kills it.
#
# Four chances to see a wedged sweep were spent killing the process and losing
# what it was waiting on. This captures that, unattended: the full process
# table, the descendant tree of every `linix`, a measured CPU delta so "idle"
# is a number rather than an impression, and each thread's wait reason.
#
# ASCII only, deliberately: a .ps1 written from bash with a non-ASCII byte in
# it fails to parse with no useful message, and this runs unattended.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File stall-snapshot.ps1 `
#       -OutFile stall-1.txt [-SampleGapMs 4000]
# ============================================================================
param(
    [Parameter(Mandatory = $true)][string]$OutFile,
    [int]$SampleGapMs = 4000,
    [string]$Note = ""
)

$ErrorActionPreference = "Continue"

# Managers LiNix drives, plus the shells it drives them through. A child that
# outlives its parent gets reparented and leaves the linix tree, so these are
# collected by name as well as by descent.
$WATCH = @(
    "linix", "scoop", "winget", "choco", "chocolatey", "powershell", "pwsh",
    "cmd", "conhost", "msiexec", "git", "bash", "sh", "node", "python",
    "curl", "wget", "tar", "7z", "openconsole", "WindowsPackageManagerServer"
)

function Get-ProcTable {
    Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Select-Object ProcessId, ParentProcessId, Name, CommandLine,
                      CreationDate, UserModeTime, KernelModeTime
}

# Two samples, because a process that is merely slow and one that is stuck look
# identical in a single frame. The gap is what tells them apart.
$first = Get-ProcTable
Start-Sleep -Milliseconds $SampleGapMs
$second = Get-ProcTable

$cpuBefore = @{}
foreach ($p in $first) {
    $cpuBefore[[int]$p.ProcessId] = [double]$p.UserModeTime + [double]$p.KernelModeTime
}

$byId = @{}
$kids = @{}
foreach ($p in $second) {
    $id = [int]$p.ProcessId
    $byId[$id] = $p
    $pp = [int]$p.ParentProcessId
    if (-not $kids.ContainsKey($pp)) { $kids[$pp] = New-Object System.Collections.ArrayList }
    [void]$kids[$pp].Add($id)
}

function Get-Descendants([int]$root) {
    $seen = New-Object System.Collections.ArrayList
    $queue = New-Object System.Collections.Queue
    $queue.Enqueue($root)
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        if ($seen -contains $cur) { continue }   # a reused PID must not loop us
        [void]$seen.Add($cur)
        if ($kids.ContainsKey($cur)) {
            foreach ($k in $kids[$cur]) { $queue.Enqueue($k) }
        }
    }
    # The comma matters: PowerShell unrolls a returned collection, and a
    # single-element tree would come back as a bare int.
    return ,$seen
}

function Cpu-Delta([int]$id) {
    $p = $byId[$id]
    if ($null -eq $p) { return -1 }
    $now = [double]$p.UserModeTime + [double]$p.KernelModeTime
    $was = $cpuBefore[$id]
    if ($null -eq $was) { return -1 }           # started during the gap
    # 100ns units -> milliseconds of CPU burned across the sample window.
    return [math]::Round(($now - $was) / 10000.0, 1)
}

function Age-Seconds([int]$id) {
    $p = $byId[$id]
    if ($null -eq $p -or $null -eq $p.CreationDate) { return -1 }
    return [math]::Round(((Get-Date) - $p.CreationDate).TotalSeconds, 0)
}

function Trim-Cmd([string]$c) {
    if ([string]::IsNullOrEmpty($c)) { return "(no command line)" }
    $c = $c -replace "\s+", " "
    if ($c.Length -gt 300) { return $c.Substring(0, 300) + "..." }
    return $c
}

$out = New-Object System.Collections.ArrayList
function Emit([string]$s) { [void]$out.Add($s) }

Emit "=============================================================="
Emit ("STALL SNAPSHOT  " + (Get-Date -Format "yyyy-MM-dd HH:mm:ss"))
if ($Note -ne "") { Emit ("note: " + $Note) }
Emit ("cpu sample window: " + $SampleGapMs + "ms")
Emit "=============================================================="

$roots = @($second | Where-Object { $_.Name -eq "linix.exe" } | ForEach-Object { [int]$_.ProcessId })

if ($roots.Count -eq 0) {
    Emit ""
    Emit "NO linix.exe IS RUNNING."
    Emit "Whatever the sweep is waiting on, it is not a LiNix process. The"
    Emit "watched-name table below is the whole of the evidence this run gets."
} else {
    Emit ""
    Emit ("linix processes: " + $roots.Count)
}

# --- The child list. The one thing four earlier opportunities threw away. ---
foreach ($r in $roots) {
    $tree = Get-Descendants $r
    Emit ""
    Emit "--------------------------------------------------------------"
    Emit ("LINIX PID " + $r + "   (tree of " + $tree.Count + " process(es))")
    Emit "--------------------------------------------------------------"
    foreach ($id in $tree) {
        $p = $byId[$id]
        if ($null -eq $p) { continue }
        $depth = 0
        $walk = $id
        while ($walk -ne $r -and $depth -lt 12) {
            $wp = $byId[$walk]
            if ($null -eq $wp) { break }
            $walk = [int]$wp.ParentProcessId
            $depth++
        }
        $indent = "  " * ($depth + 1)
        Emit ($indent + "pid=" + $id + " ppid=" + $p.ParentProcessId +
              " name=" + $p.Name +
              " cpuMsInWindow=" + (Cpu-Delta $id) +
              " ageSec=" + (Age-Seconds $id))
        Emit ($indent + "    cmd: " + (Trim-Cmd $p.CommandLine))
    }

    # Wait reasons. A process at zero CPU is blocked; this says on what.
    # `UserRequest` on every thread with a live child below is a parent waiting
    # for that child. A leaf at zero CPU with no children is the one to read
    # the command line of.
    foreach ($id in $tree) {
        $proc = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($null -eq $proc) { continue }
        $tsum = @{}
        foreach ($t in $proc.Threads) {
            $key = [string]$t.ThreadState + "/" + [string]$t.WaitReason
            if ($tsum.ContainsKey($key)) { $tsum[$key] = $tsum[$key] + 1 }
            else { $tsum[$key] = 1 }
        }
        $parts = @()
        foreach ($k in ($tsum.Keys | Sort-Object)) { $parts += ($k + " x" + $tsum[$k]) }
        Emit ("  threads pid=" + $id + " (" + $proc.ProcessName + "): " + ($parts -join ", "))
    }
}

# --- Everything with a watched name, tree or no tree. ------------------------
Emit ""
Emit "--------------------------------------------------------------"
Emit "ALL WATCHED-NAME PROCESSES (including any orphaned by a kill)"
Emit "--------------------------------------------------------------"
foreach ($p in ($second | Sort-Object Name, ProcessId)) {
    $bare = $p.Name -replace "\.exe$", ""
    if ($WATCH -notcontains $bare) { continue }
    $id = [int]$p.ProcessId
    $childCount = 0
    if ($kids.ContainsKey($id)) { $childCount = $kids[$id].Count }
    Emit ("pid=" + $id + " ppid=" + $p.ParentProcessId + " name=" + $p.Name +
          " children=" + $childCount +
          " cpuMsInWindow=" + (Cpu-Delta $id) + " ageSec=" + (Age-Seconds $id))
    Emit ("    cmd: " + (Trim-Cmd $p.CommandLine))
}

Emit ""
Emit ("END SNAPSHOT " + (Get-Date -Format "HH:mm:ss"))
Emit ""

$out | Out-File -FilePath $OutFile -Append -Encoding utf8
