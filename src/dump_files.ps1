# Define the output file
$outputFile = "project_dump.txt"
# Clear the file if it already exists
Clear-Content $outputFile -ErrorAction SilentlyContinue

# Get .rs and .toml files recursively
Get-ChildItem -Recurse -Include *.rs, *.toml | ForEach-Object {
    Add-Content $outputFile "`n--- FILE: $($_.FullName) ---`n"
    Get-Content $_.FullName | Add-Content $outputFile
    Add-Content $outputFile "`n------------------------------"
}

Write-Host "Done! All contents saved to $outputFile"
