[CmdletBinding()]
param(
    [string]$SrtVersion,
    [string]$SrtWinPath,
    [string]$OllamaHost,
    [int]$OllamaPort
)

$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force
$proofConfig = Get-SpikeProofConfig
if (-not $PSBoundParameters.ContainsKey('SrtVersion')) { $SrtVersion = $proofConfig.SrtVersion }
if (-not $PSBoundParameters.ContainsKey('SrtWinPath')) { $SrtWinPath = $proofConfig.SrtWinPath }
if (-not $PSBoundParameters.ContainsKey('OllamaHost')) { $OllamaHost = ([uri]$proofConfig.OllamaOrigin).Host }
if (-not $PSBoundParameters.ContainsKey('OllamaPort')) { $OllamaPort = ([uri]$proofConfig.OllamaOrigin).Port }
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$probeScript = (Resolve-Path (Join-Path $PSScriptRoot 'probe.mjs')).Path
$sourceDocx = (Resolve-Path (Join-Path $repoRoot 'tests\fixtures\spike-001-original.docx')).Path
$nodePath = (Get-Command node.exe -ErrorAction Stop).Source
$srtCli = Join-Path $repoRoot 'node_modules\@anthropic-ai\sandbox-runtime\dist\cli.js'

if (-not (Test-Path -LiteralPath $srtCli -PathType Leaf)) {
    throw "Pinned SRT package is missing. Run npm.cmd install first."
}
if (-not (Test-Path -LiteralPath $SrtWinPath -PathType Leaf)) {
    throw "Pinned SRT Windows helper is missing at $SrtWinPath"
}

$srtPackageJson = Join-Path $repoRoot 'node_modules\@anthropic-ai\sandbox-runtime\package.json'
$installedVersion = & $nodePath -p "require(process.argv[1]).version" $srtPackageJson
if ($installedVersion -ne $SrtVersion) {
    throw "Expected SRT $SrtVersion but found $installedVersion"
}

$tempRoot = [System.IO.Path]::GetFullPath($env:TEMP).TrimEnd('\')
$runRoot = Join-Path $tempRoot ("ollama-cowork-srt-proof-" + [guid]::NewGuid().ToString('N'))
$allowedOutput = Join-Path $runRoot 'allowed-output'
$outsideDirectory = Join-Path $runRoot 'outside-denied'
$outsideReadableCanary = Join-Path $outsideDirectory 'read-canary.txt'
$outsideWriteTarget = Join-Path $outsideDirectory 'write-canary.txt'
$childWriteTarget = Join-Path $outsideDirectory 'child-write-canary.txt'
$timeoutWriteTarget = Join-Path $allowedOutput 'timeout-survivor-canary.txt'
$settingsPath = Join-Path $runRoot 'srt-settings.json'
$requestPath = Join-Path $runRoot 'proof-request.json'
$coordinatorScript = (Resolve-Path (Join-Path $PSScriptRoot 'coordinator.mjs')).Path
$reportPath = Join-Path $repoRoot 'docs\test-plans\spike-001-srt-proof-result.json'

New-Item -ItemType Directory -Path $allowedOutput,$outsideDirectory -Force | Out-Null
[System.IO.File]::WriteAllText($outsideReadableCanary, 'SRT_OUTSIDE_READ_CANARY')

$settings = @{
    network = @{
        allowedDomains = @()
        deniedDomains = @()
        allowLocalBinding = $false
    }
    filesystem = @{
        # Windows SRT runs as a distinct account with no inherent access to
        # the caller's profile. A broad DENY on the profile prevents traversal
        # to an explicitly allowed nested workspace, so rely on default SID
        # isolation and grant only the precise read roots below.
        denyRead = @()
        allowRead = @($repoRoot, $allowedOutput, (Split-Path $nodePath -Parent))
        allowWrite = @($allowedOutput)
        denyWrite = @($sourceDocx, $outsideDirectory)
    }
    windows = @{
        srtWin = @{
            path = $SrtWinPath
        }
    }
} | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($settingsPath, $settings)

$request = @{
    srtVersion = $installedVersion
    repoRoot = $repoRoot
    nodePath = $nodePath
    probeScript = $probeScript
    sourceDocx = $sourceDocx
    outsideWriteTarget = $outsideWriteTarget
    childWriteTarget = $childWriteTarget
    timeoutWriteTarget = $timeoutWriteTarget
    postResetWaitMs = 4000
    reportPath = $reportPath
    settings = $settings | ConvertFrom-Json
    probes = @(
        @{ name = 'read selected DOCX'; expectSuccess = $true; args = @('read', $sourceDocx) }
        @{ name = 'write controlled output'; expectSuccess = $true; args = @('write', (Join-Path $allowedOutput 'allowed.txt')) }
        @{ name = 'read outside workspace'; expectSuccess = $false; args = @('read', $outsideReadableCanary) }
        @{ name = 'write outside output'; expectSuccess = $false; args = @('write', $outsideWriteTarget) }
        @{ name = 'modify original DOCX'; expectSuccess = $false; args = @('modify', $sourceDocx) }
        @{ name = 'HTTP to private Ollama'; expectSuccess = $false; args = @('http', "http://${OllamaHost}:${OllamaPort}/api/version") }
        @{ name = 'direct TCP without proxy environment'; expectSuccess = $false; args = @('tcp', $OllamaHost, $OllamaPort.ToString()) }
        @{ name = 'child process write outside output'; expectSuccess = $false; args = @('child-write', $childWriteTarget) }
        @{ name = 'timeout terminates sandbox process tree'; expectTimeout = $true; timeoutMs = 750; args = @('sleep-write', $timeoutWriteTarget, '3000') }
    )
} | ConvertTo-Json -Depth 12
[System.IO.File]::WriteAllText($requestPath, $request)

try {
    New-Item -ItemType Directory -Path (Split-Path $reportPath -Parent) -Force | Out-Null
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $nodePath $coordinatorScript $requestPath
        $coordinatorExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }

    $report = Get-Content -Raw $reportPath | ConvertFrom-Json
    $report.results | Format-Table name,expected,exit_code,timed_out,probe_reached,passed -AutoSize
    Write-Host "Canaries intact: $($report.canaries_intact)"
    Write-Host "Reset error: $($report.reset_error)"
    Write-Host "Report: $reportPath"
    exit $coordinatorExitCode
}
finally {
    $resolvedRunRoot = [System.IO.Path]::GetFullPath($runRoot)
    if ($resolvedRunRoot.StartsWith($tempRoot + '\ollama-cowork-srt-proof-', [System.StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedRunRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
    else {
        Write-Warning "Refusing to remove unexpected proof path: $resolvedRunRoot"
    }
}
