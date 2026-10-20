# Installs `hb` and `hebra-server` on Windows from the latest GitHub release
# (or $env:HEBRA_VERSION), after checking the archive against SHA256SUMS.
#
#   irm https://raw.githubusercontent.com/s-pl/hebra/main/scripts/install.ps1 | iex
#
# $env:HEBRA_INSTALL_DIR chooses where (default %LOCALAPPDATA%\Programs\hebra);
# $env:HEBRA_NO_PATH=1 leaves the user's PATH alone.
$ErrorActionPreference = 'Stop'

$repo = 's-pl/hebra'
$dir = if ($env:HEBRA_INSTALL_DIR) { $env:HEBRA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\hebra' }
$version = $env:HEBRA_VERSION
if (-not $version) {
    $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
}

$name = "hebra-$version-x86_64-pc-windows-msvc"
$base = "https://github.com/$repo/releases/download/$version"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("hebra-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading Hebra $version for Windows x64"
    Invoke-WebRequest "$base/$name.zip" -OutFile "$tmp\$name.zip" -UseBasicParsing
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS" -UseBasicParsing
    $line = Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match " $([regex]::Escape("$name.zip"))$" }
    $expected = ($line -split ' ')[0]
    $actual = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) { throw "checksum mismatch for $name.zip" }

    Expand-Archive "$tmp\$name.zip" -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    Copy-Item "$tmp\$name\hb.exe", "$tmp\$name\hebra-server.exe" -Destination $dir -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $env:HEBRA_NO_PATH -and -not ($userPath -split ';' | Where-Object { $_ -eq $dir })) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
        Write-Host "Added $dir to your PATH (open a new terminal)."
    }
    Write-Host "Installed hb and hebra-server to $dir"
    & "$dir\hb.exe" --version
}
finally {
    Remove-Item -Recurse -Force $tmp
}
