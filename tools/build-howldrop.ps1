# Build howldrop.exe, the Windows listener, from howldrop/ with CMake, Ninja and
# a MinGW-w64 g++ (Strawberry Perl ships all three). CMakeLists.txt links it
# statically, so the result needs only Windows' own DLLs.
#
#   tools\build-howldrop.ps1          build howldrop\build-win\howldrop.exe
#   tools\build-howldrop.ps1 -Run     build, stop a running howldrop.exe, start this one
param([switch]$Run)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root 'howldrop'
$build = Join-Path $src 'build-win'
foreach ($tool in 'cmake', 'ninja', 'g++') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool not found: install CMake, Ninja and a MinGW-w64 g++ (Strawberry Perl ships all three)"
    }
}

cmake -S $src -B $build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_COMPILER=g++
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cmake --build $build --target howldrop
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$exe = Join-Path $build 'howldrop.exe'
Write-Output "built $exe"
if ($Run) {
    Get-Process howldrop -ErrorAction SilentlyContinue | Stop-Process
    Start-Process -FilePath $exe -WorkingDirectory $build
    Write-Output "started $exe"
}
