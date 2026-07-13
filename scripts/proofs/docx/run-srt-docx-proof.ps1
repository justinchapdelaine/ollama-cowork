[CmdletBinding()] param()
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force
$c=Get-SpikeProofConfig;$repo=$c.RepoRoot;$temp=[IO.Path]::GetFullPath($env:TEMP).TrimEnd('\');$root=Join-Path $temp ('ollama-cowork-docx-proof-'+[guid]::NewGuid().ToString('N'))
try {
    $out=Join-Path $root 'output';New-Item -ItemType Directory -Path $out -Force|Out-Null
    $exe=(Resolve-Path $c.DocxToolExecutable).Path;$source=(Resolve-Path (Join-Path $repo 'tests\fixtures\spike-001-original.docx')).Path;$output=Join-Path $out 'spike-001-revised.docx'
    $node=(Get-Command node.exe).Source;$coordinator=(Resolve-Path (Join-Path $PSScriptRoot 'srt-docx-coordinator.mjs')).Path;$report=Join-Path $c.EvidenceRoot 'spike-001-srt-docx-proof-result.json';$requestPath=Join-Path $out 'request.json'
    $settings=@{network=@{allowedDomains=@();deniedDomains=@();allowLocalBinding=$false};filesystem=@{denyRead=@();allowRead=@($repo,$out,(Split-Path $node -Parent));allowWrite=@($out);denyWrite=@($source)};windows=@{srtWin=@{path=$c.SrtWinPath}}}
    $request=@{cwd=$repo;executable=$exe;source=$source;output=$output;request_directory=$out;replacement=@('Revised safely inside SRT.','This is a new validated copy; the source remains immutable.');report=$report;settings=$settings}
    [IO.File]::WriteAllText($requestPath,($request|ConvertTo-Json -Depth 12));&$node $coordinator $requestPath;exit $LASTEXITCODE
} finally { if($root.StartsWith($temp+'\ollama-cowork-docx-proof-',[StringComparison]::OrdinalIgnoreCase)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue} }
