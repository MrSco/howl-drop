# Build owldrop.exe, the Windows listener, from owldrop/ with CMake, Ninja and
# a MinGW-w64 g++ (Strawberry Perl ships all three). CMakeLists.txt links it
# statically, so the result needs only Windows' own DLLs.
#
#   tools\build-owldrop.ps1          build owldrop\build-win\owldrop.exe
#   tools\build-owldrop.ps1 -Run     build, stop a running owldrop.exe, start this one
param([switch]$Run)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root 'owldrop'
$build = Join-Path $src 'build-win'
foreach ($tool in 'cmake', 'ninja', 'g++') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool not found: install CMake, Ninja and a MinGW-w64 g++ (Strawberry Perl ships all three)"
    }
}

cmake -S $src -B $build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_CXX_COMPILER=g++
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cmake --build $build --target owldrop
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$exe = Join-Path $build 'owldrop.exe'
Write-Output "built $exe"
if ($Run) {
    Get-Process owldrop -ErrorAction SilentlyContinue | Stop-Process
    Start-Process -FilePath $exe -WorkingDirectory $build
    Write-Output "started $exe"
}
