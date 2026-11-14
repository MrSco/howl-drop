# Compare the optional native-radio speed experiments with the same files.
# Example: tools/test-send-speed.ps1 -Mode deep -Files C:\Temp\photo.png
# Accept on the iPhone, with AirDrop set to Everyone for 10 Minutes.
# The previous sending speed, experiment flags and running app are restored.
param(
    [Parameter(Mandatory)][ValidateSet('baseline','deep','ht','ampdu','all')][string]$Mode,
    [Parameter(Mandatory)][string[]]$Files,
    [string]$Executable = (Join-Path $PSScriptRoot '../howldrop/build-win/howldrop.exe')
)
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$inputs = @($Files | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })
if ($inputs.Count -gt 256) { throw 'Choose at most 256 files or folders.' }
$state = Join-Path $env:LOCALAPPDATA 'HowlDrop'
$settings = Join-Path $state 'settings.ini'
$outbox = Join-Path $state 'outbox'
if (Get-ChildItem -LiteralPath $outbox -Filter '*.howldrop-job' -ErrorAction SilentlyContinue) {
    throw 'Finish or cancel the queued transfers before comparing speeds.'
}
$previous = @(Get-Content -LiteralPath $settings | Where-Object { $_ -match '^tx_rate=' })
$original = @(Get-Process howldrop -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.EndsWith('howldrop.exe') })
if ($original.Count -gt 1) { throw 'Close the extra HowlDrop instances first.' }
$originalExe = if ($original.Count) { $original[0].Path } else { $null }
$oldDeep = $env:HOWLDROP_DEEP_QUEUE
$oldAmpdu = $env:HOWLDROP_AMPDU
$root = Split-Path -Parent $PSScriptRoot
$report = Join-Path $root ('build/send-speed/' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $report -Force | Out-Null
function Invoke-App([string]$App, [string[]]$Arguments) {
    $quoted = @($Arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }) -join ' '
    $p = Start-Process -FilePath $App -ArgumentList $quoted -WindowStyle Hidden -Wait -PassThru `
        -RedirectStandardOutput (Join-Path $report 'command.txt') -RedirectStandardError (Join-Path $report 'error.txt')
    if ($p.ExitCode -ne 0) { throw (Get-Content -LiteralPath (Join-Path $report 'error.txt') -Raw) }
}
function Stop-App([string]$App) {
    $running = @(Get-Process howldrop -ErrorAction SilentlyContinue | Where-Object Path -EQ $App)
    if ($running.Count) {
        Invoke-App $App @('stop')
        foreach ($p in $running) { Wait-Process -Id $p.Id -Timeout 30 -ErrorAction SilentlyContinue }
        if (Get-Process howldrop -ErrorAction SilentlyContinue | Where-Object Path -EQ $App) { throw 'HowlDrop has not stopped.' }
    }
}
function Set-Rate([string[]]$RateLines) {
    $lines = @(Get-Content -LiteralPath $settings | Where-Object { $_ -notmatch '^tx_rate=' }) + $RateLines
    [IO.File]::WriteAllLines($settings, [string[]]$lines, [Text.UTF8Encoding]::new($false))
}
$job = $null
try {
    if ($originalExe) { Stop-App $originalExe }
    $rate = if ($Mode -in 'ht','all') { 'auto-ht' } else { 'auto' }
    Set-Rate @("tx_rate=$rate")
    $env:HOWLDROP_DEEP_QUEUE = if ($Mode -ne 'baseline') { '1' } else { $null }
    $env:HOWLDROP_AMPDU = if ($Mode -in 'ampdu','all') { '1' } else { $null }
    $started = [DateTime]::UtcNow
    Start-Process -FilePath $exe -WindowStyle Hidden
    $log = Join-Path $state 'radio.log'
    $up = $false
    for ($i = 0; $i -lt 120; $i++) {
        Start-Sleep -Milliseconds 250
        if ((Test-Path -LiteralPath $log) -and (Get-Item -LiteralPath $log).LastWriteTimeUtc -ge $started -and
            (Get-Content -LiteralPath $log -Raw) -match '(?m)^@howldrop\tup\t') { $up = $true; break }
    }
    if (-not $up) { throw 'The radio did not start; check radio.log.' }
    Write-Output "Mode ${Mode}: accept the test on your iPhone."
    Invoke-App $exe (@('iphone') + $inputs)
    $accepted = $null
    for ($i = 0; $i -lt 600; $i++) {
        Start-Sleep -Milliseconds 500
        $text = Get-Content -LiteralPath $log -Raw
        if (-not $job -and $text -match '(?m)^@howldrop\toutgoing\t([0-9a-f]{32})\t') { $job = $Matches[1] }
        if (-not $job) { continue }
        if (-not $accepted -and $text -match ("(?m)^@howldrop\toutgoing-stage\t$job\taccepted; uploading")) { $accepted = [DateTime]::UtcNow }
        if ($text -match ("(?m)^@howldrop\toutgoing-done\t$job\t([^\t\r\n]+)\t([^\r\n]*)")) {
            $result = $Matches[1]; $detail = $Matches[2]
            $seconds = if ($accepted) { [math]::Round(([DateTime]::UtcNow - $accepted).TotalSeconds, 1) } else { $null }
            [pscustomobject]@{Mode=$Mode;Result=$result;UploadSeconds=$seconds;Detail=$detail;Log=(Join-Path $report 'radio.log')}
            if ($result -ne 'sent') { throw "The test ended: $result ($detail)" }
            return
        }
    }
    throw 'The test timed out.'
} finally {
    try {
        if ($job) {
            try {
                Invoke-App $exe @('cancel', $job)
                for ($i = 0; $i -lt 50 -and (Test-Path -LiteralPath (Join-Path $outbox "$job.howldrop-job")); $i++) { Start-Sleep -Milliseconds 100 }
            } catch { }
        }
        if (Test-Path -LiteralPath (Join-Path $state 'radio.log')) { Copy-Item -LiteralPath (Join-Path $state 'radio.log') -Destination (Join-Path $report 'radio.log') }
        Stop-App $exe
    } finally {
        Set-Rate $previous
        $env:HOWLDROP_DEEP_QUEUE = $oldDeep
        $env:HOWLDROP_AMPDU = $oldAmpdu
        if ($originalExe) { Start-Process -FilePath $originalExe -WindowStyle Hidden }
    }
}
