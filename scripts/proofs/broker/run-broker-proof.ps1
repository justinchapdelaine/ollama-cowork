[CmdletBinding()]param()
$ErrorActionPreference='Stop';Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force;$c=Get-SpikeProofConfig
cargo build --manifest-path (Join-Path $c.RepoRoot 'tools\docx-tool\Cargo.toml');if($LASTEXITCODE-ne0){throw 'DOCX tool build failed'}
cargo build --manifest-path (Join-Path $c.RepoRoot 'tools\broker-proof\Cargo.toml');if($LASTEXITCODE-ne0){throw 'broker proof build failed'}
$exe=(Resolve-Path $c.BrokerProofExecutable).Path;$node=(Get-Command node.exe).Source;$report=Join-Path $c.EvidenceRoot 'spike-001-broker-proof-result.json'
$output=&$exe $c.RepoRoot $node $c.SrtWinPath;if($LASTEXITCODE-ne0){$output|Out-Host;throw 'broker proof failed'};[IO.File]::WriteAllLines($report,$output);$output|Out-Host
