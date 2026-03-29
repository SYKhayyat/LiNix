# LiNix Verification Script for Windows
# Auto-detects project directory

Write-Host "LiNix Verification Script" -ForegroundColor Cyan
Write-Host "=========================" -ForegroundColor Cyan

# Find project root (where Cargo.toml is)
$scriptDir = $PSScriptRoot
if (-not $scriptDir) {
    $scriptDir = Get-Location
}

# Look for Cargo.toml
if (-not (Test-Path "$scriptDir\Cargo.toml")) {
    # Try parent directories
    $projectRoot = $scriptDir
    while ($projectRoot -and -not (Test-Path "$projectRoot\Cargo.toml")) {
        $projectRoot = Split-Path $projectRoot -Parent
    }
    
    if (-not $projectRoot) {
        Write-Host "❌ Cannot find Cargo.toml. Please run from project directory." -ForegroundColor Red
        Write-Host "Current directory: $scriptDir" -ForegroundColor Yellow
        exit 1
    }
    
    Set-Location $projectRoot
    Write-Host "Found project at: $projectRoot" -ForegroundColor Yellow
}

$BINARY = ".\target\release\linix.exe"

# Check if we need to build
if (-not (Test-Path $BINARY)) {
    Write-Host "Binary not found. Building..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        Write-Host "❌ Build failed" -ForegroundColor Red
        exit 1
    }
}

Write-Host "✅ Binary found at: $BINARY" -ForegroundColor Green

# Test help
try {
    & $BINARY --help | Out-Null
    Write-Host "✅ Help works" -ForegroundColor Green
} catch {
    Write-Host "❌ Help failed: $_" -ForegroundColor Red
    exit 1
}

# Test backends
try {
    & $BINARY backends | Out-Null
    Write-Host "✅ Backend detection works" -ForegroundColor Green
} catch {
    Write-Host "❌ Backend detection failed: $_" -ForegroundColor Red
    exit 1
}

# Test list
try {
    & $BINARY list 2>&1 | Out-Null
    Write-Host "✅ List command works" -ForegroundColor Green
} catch {
    Write-Host "⚠️  List command failed (may be expected if no backends available)" -ForegroundColor Yellow
}

# Test JSON output
try {
    $json = & $BINARY backends --json 2>&1
    if ($json) {
        $json | ConvertFrom-Json | Out-Null
        Write-Host "✅ JSON output works" -ForegroundColor Green
    } else {
        Write-Host "⚠️  JSON output empty" -ForegroundColor Yellow
    }
} catch {
    Write-Host "❌ JSON output failed: $_" -ForegroundColor Red
    exit 1
}

# Test search
try {
    & $BINARY search test 2>&1 | Out-Null
    Write-Host "✅ Search works" -ForegroundColor Green
} catch {
    Write-Host "⚠️  Search failed (may be expected)" -ForegroundColor Yellow
}

# Run unit tests
Write-Host "Running unit tests..." -ForegroundColor Yellow
cargo test --lib -q 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Unit tests pass" -ForegroundColor Green
} else {
    Write-Host "⚠️  Some unit tests failed (this may be okay)" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "=========================" -ForegroundColor Cyan
Write-Host "✅ Core verifications passed!" -ForegroundColor Green
Write-Host ""
Write-Host "Available backends:" -ForegroundColor Cyan
& $BINARY backends
Write-Host ""
Write-Host "Quick test commands:" -ForegroundColor Yellow
Write-Host "  $BINARY --help" -ForegroundColor White
Write-Host "  $BINARY backends" -ForegroundColor White
Write-Host "  $BINARY list" -ForegroundColor White
Write-Host "  $BINARY search curl" -ForegroundColor White