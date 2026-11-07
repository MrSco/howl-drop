# HowlDrop: test a USB Wi-Fi adapter, and tell us how it went.
#
# In PowerShell (Start, type "powershell", Enter), as yourself, not as
# administrator:
#
#   irm https://raw.githubusercontent.com/MrSco/howl-drop/main/tools/test-adapter.ps1 | iex
#
# It finds the USB Wi-Fi adapters plugged in and says whether HowlDrop for
# Windows can drive each. If it can, it walks through a photo from the iPhone
# and one back, with HowlDrop doing the AirDrop. Then it writes a report and
# opens a GitHub issue with it filled in: nothing goes anywhere until you
# press Create there.
#
# The report holds the adapter's name, USB ID and Windows driver, the Windows
# and HowlDrop versions, and how each step went, sizes and times. No names
# (the PC's, the iPhone's, yours), serial numbers, addresses or file names.
#
# Plain ASCII on purpose: Windows PowerShell reads a downloaded script as
# Latin-1. Nothing here may call exit, which would close the window it was
# pasted into.

& {
$ErrorActionPreference = 'Stop'
$Repo = 'MrSco/howl-drop'

# The MT7612U's USB IDs, as USB_IDS in crates/mt7612u/src/lib.rs
# (tests/test_test_adapter.py checks they match). 0e8d:7612 is the tested one.
$Mt7612uIds = @(
    '045e:02e6', '045e:02fe', '0471:2126', '0471:7600', '056e:400a',
    '057c:8503', '0846:9014', '0846:9053', '0b05:17eb', '0b05:180b',
    '0b05:1833', '0e8d:7612', '0e8d:7632', '2357:0137', '2c4e:0103'
)
$TestedIds = @('0e8d:7612')

$HowlDropDir = Join-Path $env:LOCALAPPDATA 'HowlDrop'
$RadioLog = Join-Path $HowlDropDir 'radio.log'
# What the test has read so far. One table, so that nothing is left behind
# in the PowerShell session it was pasted into.
$S = @{
    logAt           = 0
    logRest         = ''
    seen            = New-Object System.Collections.Generic.List[string]
    agentAdapter    = ''
    howldropVersion = ''
    # Each AirDrop device's slots, by its address, from HowlDrop's
    # "schedule" lines during the step under way.
    schedules       = [ordered]@{}
}
$TestPhoto = 'HowlDrop test.jpg'

function Say([string]$text, [string]$color = 'Gray') { Write-Host $text -ForegroundColor $color }
function Title([string]$text) { Write-Host ''; Write-Host $text -ForegroundColor Cyan }
function Good([string]$text) { Write-Host "  OK  $text" -ForegroundColor Green }
function Bad([string]$text) { Write-Host "  !!  $text" -ForegroundColor Yellow }

function Ask([string]$question, [string]$default = '') {
    $answer = Read-Host $question
    if ([string]::IsNullOrWhiteSpace($answer)) { return $default }
    return $answer.Trim()
}

function Yes([string]$question, [bool]$default = $true) {
    $hint = if ($default) { '[Y/n]' } else { '[y/N]' }
    $answer = Ask "$question $hint"
    if ($answer -eq '') { return $default }
    return $answer -match '^[yY]'
}

# --- The adapters -----------------------------------------------------------

function Prop($id, [string]$key) {
    try {
        $p = Get-PnpDeviceProperty -InstanceId $id -KeyName $key -ErrorAction Stop
        if ($p.Data) { return ([string]$p.Data).Trim() }
    } catch {}
    return ''
}

# USB devices that are, or look like, Wi-Fi adapters: Windows' network
# adapters, an MT7612U on any driver (HowlDrop's setup puts it on WinUSB,
# where Windows calls it a plain USB device), and anything without a working
# driver whose name says Wi-Fi. One entry per USB ID.
function Find-Adapters {
    $all = @(Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '^USB\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}' })
    $found = [ordered]@{}
    foreach ($d in $all) {
        $null = $d.InstanceId -match '^USB\\VID_([0-9A-F]{4})&PID_([0-9A-F]{4})'
        $usbId = ($Matches[1] + ':' + $Matches[2]).ToLower()
        $wifiName = $d.FriendlyName -match 'Wi-?Fi|WLAN|802\.11'
        $looksLike = ($d.Class -eq 'Net') -or ($Mt7612uIds -contains $usbId) -or
            ($wifiName -and ($d.Class -in @('USBDevice', 'Unknown', '', $null)))
        if (-not $looksLike) { continue }
        # The network adapter itself says the most; else the first found.
        if (-not $found.Contains($usbId) -or $d.Class -eq 'Net') { $found[$usbId] = $d }
    }
    foreach ($usbId in $found.Keys) {
        $d = $found[$usbId]
        $usbParent = $all | Where-Object { $_.InstanceId -match ('^USB\\VID_' + $usbId.Replace(':', '&PID_') + '\\') } |
            Select-Object -First 1
        $productString = if ($usbParent) { Prop $usbParent.InstanceId 'DEVPKEY_Device_BusReportedDeviceDesc' } else { '' }
        $chip = ''
        if ($Mt7612uIds -contains $usbId) { $chip = 'MT7612U' }
        [pscustomobject]@{
            UsbId          = $usbId
            Name           = ([string]$d.FriendlyName).Trim()
            ProductString  = $productString
            WindowsClass   = [string]$d.Class
            WindowsDriver  = Prop $d.InstanceId 'DEVPKEY_Device_Service'
            DriverProvider = Prop $d.InstanceId 'DEVPKEY_Device_DriverProvider'
            DriverVersion  = Prop $d.InstanceId 'DEVPKEY_Device_DriverVersion'
            Manufacturer   = Prop $d.InstanceId 'DEVPKEY_Device_Manufacturer'
            Working        = [string]$d.Status -eq 'OK'
            Chip           = $chip
        }
    }
}

function Find-BuiltIn {
    @(Get-PnpDevice -PresentOnly -Class Net -ErrorAction SilentlyContinue |
        Where-Object { $_.InstanceId -match '^PCI\\' -and $_.FriendlyName -match 'Wi-?Fi|Wireless|WLAN|802\.11' } |
        ForEach-Object { $_.FriendlyName })
}

function Describe($a) {
    $name = if ($a.Name) { $a.Name } else { 'A USB device' }
    if ($a.ProductString -and $a.ProductString -ne $a.Name) { $name += " (it calls itself `"$($a.ProductString)`")" }
    return "$name, USB ID $($a.UsbId)"
}

# --- HowlDrop's radio log -------------------------------------------------

# Reads what HowlDrop's radio adds to radio.log from now on.
function Start-Log {
    $S.logAt = if (Test-Path $RadioLog) { (Get-Item $RadioLog).Length } else { 0 }
    $S.logRest = ''
}
function Read-Log {
    if (-not (Test-Path $RadioLog)) { return @() }
    $length = (Get-Item $RadioLog).Length
    # Past 1 MB HowlDrop starts it over.
    if ($length -lt $S.logAt) { $S.logAt = 0; $S.logRest = '' }
    if ($length -eq $S.logAt) { return @() }
    $f = [System.IO.File]::Open($RadioLog, 'Open', 'Read', 'ReadWrite, Delete')
    try {
        $null = $f.Seek($S.logAt, 'Begin')
        $bytes = New-Object byte[] ($length - $S.logAt)
        $n = $f.Read($bytes, 0, $bytes.Length)
        $S.logAt += $n
    } finally { $f.Close() }
    $text = $S.logRest + [System.Text.Encoding]::UTF8.GetString($bytes, 0, $n)
    $lines = $text -split "`r?`n"
    $S.logRest = $lines[-1]
    if ($lines.Count -gt 1) { return $lines[0..($lines.Count - 2)] }
    return @()
}

# The log's last lines worth reading, with what could say who or where taken
# out: for a report on what went wrong.
function Remember([string]$line) {
    if ($line -match '^ ?INFO (STATS|REORDER|add peer|remove peer|schedule)|^state: Downloading|^@howldrop\t(receiving|sending)') { return }
    if ($line -match '^(Receive: stored|Send: |Receive: listening|The iPhone share sheet|state: On the share sheet|Window is up)') { return }
    $line = $line -replace '([0-9a-fA-F]{2}[:-]){5}[0-9a-fA-F]{2}', 'xx:xx:xx:xx:xx:xx'
    $line = $line -replace '\[?fe80:[0-9a-fA-F:]*(%[^\])\s]*)?\]?(:\d+)?', '[address]'
    if ($env:USERPROFILE) { $line = $line -replace [regex]::Escape($env:USERPROFILE), '%USERPROFILE%' }
    if ($env:COMPUTERNAME) { $line = $line -replace [regex]::Escape($env:COMPUTERNAME), 'PC' }
    $line = $line -replace '"[^"]*"', '"..."'
    if ($line.Length -gt 200) { $line = $line.Substring(0, 200) }
    $S.seen.Add($line)
    while ($S.seen.Count -gt 25) { $S.seen.RemoveAt(0) }
}

# "3.3 MB in 1.0 s (3.2 MB/s)": the size and time at the end of a line.
function Timing([string]$line) {
    $m = [regex]::Matches($line, '(\d+(?:\.\d+)? (?:bytes|KB|MB|GB)) in (under a second|\d+(?:\.\d+)? s)')
    if ($m.Count -eq 0) { return $null }
    $last = $m[$m.Count - 1]
    return [ordered]@{ size = $last.Groups[1].Value; time = $last.Groups[2].Value }
}

# Bytes a second from Timing's size and time; $null under a second.
function Rate($t) {
    if (-not $t -or $t.time -notmatch '^([\d.]+) s$') { return $null }
    $secs = [double]$Matches[1]
    if ($t.size -notmatch '^([\d.]+) (bytes|KB|MB|GB)$') { return $null }
    $unit = @{ bytes = 1; KB = 1e3; MB = 1e6; GB = 1e9 }[$Matches[2]]
    return [double]$Matches[1] * $unit / $secs
}

# Half a megabyte or more at under 1 MB/s: slower than a working stick goes,
# and most often the iPhone busy on its own Wi-Fi.
function Is-Slow($t) {
    $rate = Rate $t
    if ($null -eq $rate) { return $false }
    $secs = [double]($t.time -replace ' s$', '')
    return $rate -lt 1e6 -and $rate * $secs -ge 5e5
}

# What the AirDrop devices said about their slots during the step, without
# their addresses.
function Schedules {
    @($S.schedules.Values | Select-Object -Unique | Select-Object -Last 4)
}

# HowlDrop's state now: the last "state:" line, which is what its menu says.
function Last-State {
    if (-not (Test-Path $RadioLog)) { return '' }
    $f = [System.IO.File]::Open($RadioLog, 'Open', 'Read', 'ReadWrite, Delete')
    try { $all = (New-Object System.IO.StreamReader($f)).ReadToEnd() } finally { $f.Close() }
    $state = ''
    foreach ($line in ($all -split "`r?`n")) {
        if ($line.StartsWith('state: ')) { $state = $line.Substring(7) }
        if ($line -match '^HowlDrop (\S+): AirDrop on this PC') { $S.howldropVersion = $Matches[1] }
        if ($line -match "^@howldrop\tadapter\t") { $S.agentAdapter = $line }
    }
    return $state
}

# Waits for a line that $done says ends the step, showing HowlDrop's state
# meanwhile. S skips. Returns the line, or '' when skipped or timed out.
function Wait-For([scriptblock]$done, [int]$minutes) {
    $until = (Get-Date).AddMinutes($minutes)
    $lastState = ''
    Say "  (Waiting up to $minutes minutes. Press S to skip this step.)" DarkGray
    while ((Get-Date) -lt $until) {
        foreach ($line in (Read-Log)) {
            Remember $line
            if ($line -match "^@howldrop\tadapter\t") { $S.agentAdapter = $line }
            if ($line -match '^ ?INFO schedule ([0-9a-f:]{17}): [0-9 ]+ \(([^)]*)\); network ([^;]*);') {
                $S.schedules[$Matches[1]] = "$($Matches[2]); network $($Matches[3])"
            }
            if ($line -match "^@howldrop\tstick\tneeds-winusb") {
                Say '  Windows asks once to let HowlDrop set up the stick: say Yes.' White
            }
            if ($line.StartsWith('state: ') -and -not $line.StartsWith('state: Downloading')) {
                $s = $line.Substring(7)
                if ($s -ne $lastState) { Say "  HowlDrop: $s" DarkGray; $lastState = $s }
            }
            if (& $done $line) { return $line }
        }
        try {
            if ([Console]::KeyAvailable) {
                $k = [Console]::ReadKey($true)
                if ($k.Key -eq 'S') { Say '  Skipped.' DarkGray; return '' }
            }
        } catch {}
        Start-Sleep -Milliseconds 400
    }
    Bad "Nothing after $minutes minutes."
    return ''
}

# A photo of noise, about 2 MB, so the send takes long enough to time.
function New-TestPhoto([string]$path) {
    Add-Type -AssemblyName System.Drawing
    $w = 2000; $h = 1500
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try {
        $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
        $data = $bmp.LockBits($rect, 'WriteOnly', $bmp.PixelFormat)
        $bytes = New-Object byte[] ($data.Stride * $h)
        (New-Object System.Random).NextBytes($bytes)
        [System.Runtime.InteropServices.Marshal]::Copy($bytes, 0, $data.Scan0, $bytes.Length)
        $bmp.UnlockBits($data)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $font = New-Object System.Drawing.Font 'Segoe UI', 96, ([System.Drawing.FontStyle]::Bold)
        $g.FillRectangle([System.Drawing.Brushes]::White, 400, 600, 1200, 300)
        $g.DrawString('HowlDrop test', $font, [System.Drawing.Brushes]::Black, 460, 660)
        $g.Dispose()
        $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Jpeg)
    } finally { $bmp.Dispose() }
}

# --- The AirDrop test -----------------------------------------------------

function Find-HowlDrop {
    $p = Get-Process howldrop -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($p) { return 'running' }
    $exe = Join-Path $env:LOCALAPPDATA 'Programs\HowlDrop\howldrop.exe'
    if (Test-Path $exe) { return $exe }
    return ''
}

# One step's transfer, and if it was slow, a hint and the chance to try it
# again: the retry's result goes with the first.
function Again-If-Slow([scriptblock]$step, [string]$again) {
    $first = & $step
    if (-not $first.ok -or -not (Is-Slow $first)) { return $first }
    Say ''
    Bad 'That was slow: a working stick does 2 MB/s and more.'
    Say '      Most often the iPhone is busy on its own Wi-Fi (an iCloud upload, a VPN' White
    Say '      with an exit node) and gives AirDrop less time. To check: on the iPhone,' White
    Say '      open Control Center and tap the Wi-Fi icon once (AirDrop keeps working).' White
    if (-not (Yes 'Try this step again?')) { return $first }
    Say $again White
    $first.retry = & $step
    return $first
}

function Receive-Once {
    $S.schedules.Clear()
    $line = Wait-For { param($l) $l.StartsWith('Receive: stored ') -or $l.StartsWith('Receive: the transfer failed') } 5
    $out = [ordered]@{ ok = $null }
    if ($line.StartsWith('Receive: stored ')) {
        $t = Timing $line
        $out.ok = $true
        if ($t) { $out.size = $t.size; $out.time = $t.time; Good "Received: $($t.size) in $($t.time)." }
        else { Good 'Received.' }
    } elseif ($line) {
        Bad 'The transfer failed.'
        $out.ok = $false
    } else {
        $out.seen_by_iphone = Yes 'Did the iPhone see this PC at all?' $false
    }
    $out.iphone_slots = Schedules
    return $out
}

function Send-Once {
    $S.schedules.Clear()
    $outbox = Join-Path $HowlDropDir 'outbox'
    $null = New-Item -ItemType Directory -Force $outbox
    # A dotfile until it is whole: HowlDrop sends what is in its outbox and
    # leaves dotfiles alone.
    $temp = Join-Path $outbox ".$TestPhoto"
    $made = $false
    try {
        New-TestPhoto $temp
        Move-Item -Force $temp (Join-Path $outbox $TestPhoto)
        $made = $true
    } catch {
        Bad "Could not make a test photo ($($_.Exception.Message))."
        Say 'Instead, click the HowlDrop icon and drag any photo onto its window.' White
    }
    if ($made) {
        Say "HowlDrop is sending `"$TestPhoto`" (a photo of noise, about 2 MB)." White
        Say 'When the iPhone asks, tap Accept.' White
    }
    $pattern = if ($made) { '^Send: ' + [regex]::Escape($TestPhoto) + ': (sent|not sent)' } else { '^Send: .*: (sent|not sent)' }
    $line = Wait-For { param($l) $l -match $pattern } 5
    $out = [ordered]@{ ok = $null }
    if ($line -match '^Send: .*: sent') {
        $t = Timing $line
        $out.ok = $true
        if ($t) { $out.size = $t.size; $out.time = $t.time; Good "Sent: $($t.size) in $($t.time)." }
        else { Good 'Sent.' }
    } elseif ($line) {
        $why = ($line -replace '^Send: .*: not sent: ', '') -replace '; moved to .*$', ''
        Bad "Not sent: $why"
        $out.ok = $false
        $out.why = $why
    }
    if ($line -eq '') {
        Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $outbox $TestPhoto)
    }
    $out.iphone_slots = Schedules
    return $out
}

# How a step ended: a retry that went through counts.
function Final($step) {
    if ($step.retry) { return $step.retry.ok }
    return $step.ok
}

function Test-AirDrop($result) {
    Title 'Step 2 of 4: HowlDrop'
    $where = Find-HowlDrop
    while ($where -ne 'running') {
        if ($where) {
            if (Yes "HowlDrop isn't running. Start it?") { Start-Process $where; Start-Sleep 3 }
            else { $result.test = 'skipped'; return }
        } else {
            Say 'HowlDrop is one program, howldrop.exe, with nothing to install:'
            Say "  1. Download it from https://github.com/$Repo/releases/latest"
            Say '  2. Put it somewhere it can stay, say %LOCALAPPDATA%\Programs\HowlDrop'
            Say '  3. Double-click it: its icon shows up by the clock.'
            if (Yes 'Open the download page?') { Start-Process "https://github.com/$Repo/releases/latest" }
            $go = Ask 'Press Enter once HowlDrop is running (or type S to skip the AirDrop test)'
            if ($go -match '^[sS]') { $result.test = 'skipped'; return }
        }
        $where = Find-HowlDrop
    }
    Good 'HowlDrop is running.'

    Start-Log
    $state = Last-State
    if ($state -notmatch '^On the share sheet as (.+)$') {
        Say ''
        Say 'Turn on AirDrop on this PC: right-click the HowlDrop icon by the clock' White
        Say '(it may be under the ^ arrow) and choose "AirDrop on this PC".' White
        Say 'The first time, it downloads its AirDrop part (about 3.5 MB), and Windows' White
        Say 'asks once to let it set up the stick: say Yes.' White
        $line = Wait-For { param($l) $l -match '^state: On the share sheet as ' -or $l -match "^@howldrop\tstopped" } 4
        if ($line -notmatch '^state: On the share sheet as (.+)$') {
            Bad 'HowlDrop did not get on the share sheet with this stick.'
            $result.on_share_sheet = $false
            $result.test = 'failed'
            return
        }
        $state = $line.Substring(7)
        $null = $state -match '^On the share sheet as (.+)$'
    }
    $pcName = $Matches[1]
    $null = Last-State
    Good "HowlDrop is on the share sheet as `"$pcName`"."
    $result.on_share_sheet = $true

    Title 'Step 3 of 4: from the iPhone to this PC'
    Say 'On the iPhone, open a photo (or a video, for a better speed test), tap Share,' White
    Say "then AirDrop, and pick `"$pcName`". If a card on the PC asks, click Accept." White
    $result.receive = Again-If-Slow { Receive-Once } 'AirDrop the photo to the PC again.'

    Title 'Step 4 of 4: from this PC to the iPhone'
    Say "On the iPhone, set AirDrop to Everyone for 10 Minutes: Settings, General," White
    Say 'AirDrop. Keep the iPhone unlocked and near the PC.' White
    $go = Ask 'Press Enter when it is set (or type S to skip)'
    if ($go -match '^[sS]') { $result.send = [ordered]@{ ok = $null } }
    else { $result.send = Again-If-Slow { Send-Once } 'HowlDrop sends the photo again.' }

    $ok = @((Final $result.receive), (Final $result.send))
    if ($ok -notcontains $false -and $ok -notcontains $null) { $result.test = 'works' }
    elseif ($ok -contains $true) { $result.test = 'partly' }
    elseif ($ok -contains $false) { $result.test = 'failed' }
    else { $result.test = 'not finished' }
}

# --- The report -----------------------------------------------------------

function Windows-Version {
    $v = [Environment]::OSVersion.Version
    $display = ''
    try { $display = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -Name DisplayVersion).DisplayVersion } catch {}
    $name = if ($v.Build -ge 22000) { 'Windows 11' } else { 'Windows 10' }
    return ("$name $display ($($v.Major).$($v.Minor).$($v.Build))" -replace '  ', ' ')
}

function Main {
    Clear-Host
    Say 'HowlDrop adapter test' Cyan
    Say '====================='
    Say 'This checks whether your USB Wi-Fi adapter can do AirDrop with HowlDrop,'
    Say 'and makes a report you can send us in one click. Your report helps the'
    Say 'next person, and tells us which adapters to support next.'
    Say ''
    Say 'Nothing is sent anywhere until you press Create on GitHub at the end.' DarkGray

    Title 'Step 1 of 4: your adapter'
    $adapters = @()
    while ($true) {
        $adapters = @(Find-Adapters)
        if ($adapters.Count -gt 0) { break }
        Bad 'No USB Wi-Fi adapter found.'
        foreach ($b in (Find-BuiltIn)) {
            Say "  (Your PC's built-in $b can't do AirDrop on Windows: it needs a USB adapter.)" DarkGray
        }
        $go = Ask 'Plug it in and press Enter to look again (or type Q to quit)'
        if ($go -match '^[qQ]') { return }
    }
    $a = $adapters[0]
    if ($adapters.Count -gt 1) {
        Say 'Found these:'
        for ($i = 0; $i -lt $adapters.Count; $i++) { Say "  $($i + 1). $(Describe $adapters[$i])" }
        $pick = 0
        while ($pick -lt 1 -or $pick -gt $adapters.Count) {
            $answer = Ask "Which one to test? (1-$($adapters.Count))" '1'
            $null = [int]::TryParse($answer, [ref]$pick)
        }
        $a = $adapters[$pick - 1]
    }
    Say "Testing: $(Describe $a)" White

    $result = [ordered]@{
        report         = 'howldrop-adapter-test'
        version        = 1
        date           = (Get-Date).ToString('yyyy-MM-dd')
        windows        = Windows-Version
        adapter        = [ordered]@{
            name            = $a.Name
            usb_id          = $a.UsbId
            product_string  = $a.ProductString
            chip            = $a.Chip
            windows_class   = $a.WindowsClass
            windows_driver  = $a.WindowsDriver
            driver_provider = $a.DriverProvider
            driver_version  = $a.DriverVersion
            manufacturer    = $a.Manufacturer
            driver_working  = $a.Working
        }
        supported      = $false
        test           = 'not run'
        on_share_sheet = $null
        receive        = $null
        send           = $null
    }

    if ($a.Chip -eq 'MT7612U') {
        $result.supported = $true
        if ($TestedIds -contains $a.UsbId) {
            Good 'This is a MediaTek MT7612U: the chip HowlDrop for Windows drives, and this exact one is tested.'
        } else {
            Good 'This is a MediaTek MT7612U: the chip HowlDrop for Windows drives.'
            Say '      This USB ID is not tested yet, so your test is the first. Thank you!' Green
        }
        Test-AirDrop $result
    } else {
        Bad "HowlDrop for Windows can't drive this adapter yet."
        Say '      It has a driver of its own for one chip, the MediaTek MT7612U' White
        Say '      (Windows'' own Wi-Fi drivers can''t do AirDrop). Send the report anyway:' White
        Say '      it tells us which chips people have, so we know which driver to write next.' White
        $result.test = 'not supported yet'
    }

    $agent = $S.agentAdapter
    if ($agent) {
        $f = $agent -split "`t"
        $result.howldrop_adapter = [ordered]@{ driver = $f[2]; status = $f[3]; mode = $f[4]; usb_id = $(if ($f.Count -gt 5) { $f[5] } else { '' }) }
    }
    if ($S.howldropVersion) { $result.howldrop = $S.howldropVersion }
    if ($result.test -in @('failed', 'partly', 'not finished')) { $result.log = @($S.seen) }

    Title 'Last: a little about your iPhone (optional)'
    $iphone = Ask 'Which iPhone and iOS? e.g. "iPhone 15, iOS 26.1" (Enter to skip)'
    if ($iphone) { $result.iphone = $iphone }
    $notes = Ask 'Anything else we should know, e.g. where you bought it? (Enter to skip)'
    if ($notes) { $result.notes = $notes }

    $json = $result | ConvertTo-Json -Depth 6
    $file = Join-Path ([Environment]::GetFolderPath('Desktop')) ("howldrop-adapter-" + $a.UsbId.Replace(':', '-') + '.json')
    [System.IO.File]::WriteAllText($file, $json, (New-Object System.Text.UTF8Encoding $false))
    try { Set-Clipboard -Value $json } catch {}

    $how = switch ($result.test) {
        'works' { 'works both ways' }
        'partly' { if (Final $result.receive) { 'receives, does not send' } else { 'sends, does not receive' } }
        'failed' { 'does not work' }
        'not supported yet' { 'not supported yet' }
        default { 'not finished' }
    }
    $named = if ($a.Name) { $a.Name } else { 'USB adapter' }

    Title 'Your report'
    Say $json DarkGray
    Say ''
    Good "Saved on your Desktop: $(Split-Path -Leaf $file), and copied."
    Say ''
    Say 'Last step: send it. Your browser opens GitHub with the report filled in.' White
    Say 'Sign in (or make a free account), look it over, and press Create.' White
    if (-not (Yes 'Open GitHub now?')) {
        Say "Whenever you like: https://github.com/$Repo/issues/new?template=adapter-report.yml"
        Say "and paste the report (it's on your Desktop)."
        return
    }
    $q = 'template=adapter-report.yml' +
        '&title=' + [uri]::EscapeDataString("Adapter: $named ($($a.UsbId)): $how") +
        '&adapter=' + [uri]::EscapeDataString("$named ($($a.UsbId))") +
        '&result=' + [uri]::EscapeDataString($how)
    $withReport = $q + '&report=' + [uri]::EscapeDataString($json)
    # Browsers and GitHub take about 8,000 characters in an address.
    if ($withReport.Length -lt 7000) { $q = $withReport }
    else { Say 'The report is too long for the address: paste it into the Report box (it is copied).' Yellow }
    Start-Process "https://github.com/$Repo/issues/new?$q"
    Say ''
    Say 'Thank you!' Green
}

try { Main }
catch {
    Write-Host ''
    Write-Host "Sorry, the test stopped: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host "Please tell us at https://github.com/$Repo/issues/new with that line. Thank you!" -ForegroundColor Yellow
}
}
