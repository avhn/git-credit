# Install git-credit from its GitHub release on Windows, without administrator rights.
#
#   irm https://raw.githubusercontent.com/avhn/git-credit/main/install.ps1 | iex
#
# Environment:
#   GIT_CREDIT_VERSION      release to install, for example 0.1.1 (default: latest)
#   GIT_CREDIT_INSTALL_DIR  target directory (default: %LOCALAPPDATA%\Programs\git-credit)
#   GIT_CREDIT_NO_MODIFY_PATH  set to 1 to leave the user PATH unchanged
#
# The archive is verified against the release's SHA256SUMS before anything is
# installed. The install directory is added to the user PATH if it is missing.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Windows PowerShell 5.1 may default to TLS 1.0/1.1, which GitHub rejects.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'avhn/git-credit'
$installDir = if ($env:GIT_CREDIT_INSTALL_DIR) { $env:GIT_CREDIT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\git-credit' }

if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    throw "git-credit install: unsupported architecture $($env:PROCESSOR_ARCHITECTURE); only x86_64 Windows builds are published"
}

# Resolve the release tag: an explicit version, or the latest release.
if ($env:GIT_CREDIT_VERSION) {
    $tag = 'v' + $env:GIT_CREDIT_VERSION.TrimStart('v')
} else {
    $tag = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
}
if ($tag -notmatch '^v\d+\.\d+\.\d+$') {
    throw "git-credit install: could not determine a release version (got '$tag')"
}

$archive = 'git-credit-x86_64-pc-windows-msvc.zip'
$base = "https://github.com/$repo/releases/download/$tag"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("git-credit-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "Downloading git-credit $tag for x86_64-pc-windows-msvc"
    Invoke-WebRequest -UseBasicParsing "$base/$archive" -OutFile (Join-Path $tmp $archive)
    Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')

    $expected = $null
    foreach ($line in Get-Content (Join-Path $tmp 'SHA256SUMS')) {
        $parts = $line -split '\s+', 2
        if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $archive) { $expected = $parts[0].ToLowerInvariant() }
    }
    if (-not $expected) { throw "git-credit install: $archive is not listed in SHA256SUMS" }
    # Hash through .NET: Get-FileHash can be unavailable when module paths are mixed.
    $stream = [System.IO.File]::OpenRead((Join-Path $tmp $archive))
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        $actual = -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') })
    } finally {
        $stream.Dispose()
    }
    if ($expected -ne $actual) { throw "git-credit install: checksum mismatch for $archive" }

    Expand-Archive (Join-Path $tmp $archive) -DestinationPath (Join-Path $tmp 'unpacked')
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item (Join-Path $tmp 'unpacked\git-credit.exe') (Join-Path $installDir 'git-credit.exe') -Force
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$exe = Join-Path $installDir 'git-credit.exe'
Write-Host "Installed $(& $exe --version) to $exe"

# Add the install directory to the user PATH once; new terminals pick it up.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$entries = if ($userPath) { $userPath -split ';' } else { @() }
if ($env:GIT_CREDIT_NO_MODIFY_PATH -eq '1') {
    Write-Host "Leaving PATH unchanged; run $exe directly or add $installDir to PATH."
} elseif ($entries -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', (($entries + $installDir) -join ';').Trim(';'), 'User')
    Write-Host "Added $installDir to your user PATH. Open a new terminal, then run: git credit analyze"
} else {
    Write-Host 'Try it inside any git repository: git credit analyze'
}
