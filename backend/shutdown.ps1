Write-Host "Gracefully shutting down lingering processes..."

$procs = Get-Process | Where-Object { $_.Name -match "python|uvicorn|qdrant" }
if ($procs) {
    $procs | Stop-Process -Force
    Write-Host "Terminated orphaned Python/Uvicorn/Qdrant processes."
} else { Write-Host "No matching processes running." }
