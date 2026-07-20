# Full LiNix Test Suite for Windows

Write-Host "=== LiNix Full Test Suite ===" -ForegroundColor Cyan

$BINARY = ".\target\release\linix.exe"

# 1. Build (already done, but verify)
Write-Host "`n[1/12] Verifying binary..." -ForegroundColor Yellow
if (-not (Test-Path $BINARY)) {
    Write-Host "Building..." -ForegroundColor Yellow
    cargo build --release
    if ($LASTEXITCODE -ne 0) { 
        Write-Host "❌ Build failed" -ForegroundColor Red
        exit 1 
    }
}
Write-Host "✅ Binary ready" -ForegroundColor Green

# 2. Help
Write-Host "`n[2/12] Testing help..." -ForegroundColor Yellow
& $BINARY --help | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Help works" -ForegroundColor Green
} else {
    Write-Host "❌ Help failed" -ForegroundColor Red
}

# 3. Version
Write-Host "`n[3/12] Testing version..." -ForegroundColor Yellow
$version = & $BINARY --version
Write-Host "Version: $version" -ForegroundColor White
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Version works" -ForegroundColor Green
} else {
    Write-Host "❌ Version failed" -ForegroundColor Red
}

# 4. Backends
Write-Host "`n[4/12] Detecting backends..." -ForegroundColor Yellow
$backends = & $BINARY backends 2>&1
Write-Host $backends
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Backend detection works" -ForegroundColor Green
} else {
    Write-Host "❌ Backend detection failed" -ForegroundColor Red
}

# 5. List
Write-Host "`n[5/12] Listing packages (first 20)..." -ForegroundColor Yellow
& $BINARY list 2>&1 | Select-Object -First 20
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ List works" -ForegroundColor Green
} else {
    Write-Host "⚠️  List failed (may be expected if no backends)" -ForegroundColor Yellow
}

# 6. Search
Write-Host "`n[6/12] Testing search..." -ForegroundColor Yellow
& $BINARY search python 2>&1 | Select-Object -First 10
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Search works" -ForegroundColor Green
} else {
    Write-Host "⚠️  Search failed" -ForegroundColor Yellow
}

# 7. JSON output - backends
Write-Host "`n[7/12] Testing JSON output..." -ForegroundColor Yellow
$json = & $BINARY backends --json 2>&1
try {
    $parsed = $json | ConvertFrom-Json
    Write-Host "JSON backends: $($parsed -join ', ')" -ForegroundColor White
    Write-Host "✅ JSON output works" -ForegroundColor Green
} catch {
    Write-Host "❌ JSON parsing failed: $_" -ForegroundColor Red
}

# 8. JSON output - list
Write-Host "`n[8/12] Testing JSON list..." -ForegroundColor Yellow
$jsonList = & $BINARY list --json 2>&1 | ConvertFrom-Json
$count = ($jsonList | Measure-Object).Count
Write-Host "Found $count packages in JSON" -ForegroundColor White
if ($count -gt 0) {
    Write-Host "Sample package: $($jsonList[0].name) [$($jsonList[0].backend)]" -ForegroundColor White
    Write-Host "✅ JSON list works" -ForegroundColor Green
} else {
    Write-Host "⚠️  No packages in JSON output" -ForegroundColor Yellow
}

# 9. Dry-run sync
Write-Host "`n[9/12] Testing dry-run sync..." -ForegroundColor Yellow
$configPath = "$env:USERPROFILE\.config\linix-test\preferences.toml"
if (Test-Path $configPath) {
    & $BINARY --config $configPath --dry-run sync 2>&1 | Select-Object -First 10
    Write-Host "✅ Dry-run sync executed" -ForegroundColor Green
} else {
    Write-Host "⚠️  Test config not found at $configPath" -ForegroundColor Yellow
    Write-Host "   Run: .\setup-test-env.ps1" -ForegroundColor White
}

# 10. Unmanaged
Write-Host "`n[10/12] Testing unmanaged..." -ForegroundColor Yellow
if (Test-Path $configPath) {
    & $BINARY --config $configPath unmanaged 2>&1 | Select-Object -First 10
    Write-Host "✅ Unmanaged command executed" -ForegroundColor Green
} else {
    Write-Host "⚠️  Skipped (no test config)" -ForegroundColor Yellow
}

# 11. Info command
Write-Host "`n[11/12] Testing info command..." -ForegroundColor Yellow
& $BINARY info python 2>&1 | Select-Object -First 10
Write-Host "✅ Info command executed" -ForegroundColor Green

# 12. Completions
Write-Host "`n[12/12] Testing completions..." -ForegroundColor Yellow
& $BINARY completions powershell | Select-Object -First 5
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Completions work" -ForegroundColor Green
} else {
    Write-Host "⚠️  Completions failed" -ForegroundColor Yellow
}

# Summary
Write-Host "`n" + ("="*50) -ForegroundColor Cyan
Write-Host "=== Test Suite Complete ===" -ForegroundColor Green
Write-Host ("="*50) -ForegroundColor Cyan

# Show what backends are available
Write-Host "`nAvailable backends on this system:" -ForegroundColor Cyan
& $BINARY backends

Write-Host "`nNext steps:" -ForegroundColor Yellow
Write-Host "  1. Try: $BINARY list --backend <backend>" -ForegroundColor White
Write-Host "  2. Try: $BINARY search <query>" -ForegroundColor White
Write-Host "  3. Try: $BINARY info <package>" -ForegroundColor White
Write-Host "  4. Setup test environment: .\setup-test-env.ps1" -ForegroundColor White
Write-Host "  5. Try safe install: .\safe-install-test.ps1" -ForegroundColor White