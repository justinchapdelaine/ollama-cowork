[CmdletBinding()] param([int]$Port=40961)
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force
$c=Get-SpikeProofConfig;$runtime=$null;$completed=$false
try{
    $runtime=Start-OpencodeProofRuntime -Name 'opencode-server-proof' -Port $Port -Permission @{'*'='deny'} -ToolFiles @((Join-Path $PSScriptRoot 'tools\bash.ts')) -LogLevel DEBUG
    $unauth=Invoke-OpencodeWebRequest $runtime '/global/health' $false
    $doc=Invoke-OpencodeWebRequest $runtime '/doc' $true
    $config=Invoke-OpencodeWebRequest $runtime '/config' $true
    $ids=Invoke-OpencodeWebRequest $runtime '/experimental/tool/ids' $true 60
    $schemas=Invoke-OpencodeWebRequest $runtime "/experimental/tool?provider=$($c.ProviderId)&model=gemma4%3A12b" $true 60
    $listeners=Get-NetTCPConnection -OwningProcess $runtime.Process.Id -State Listen -ErrorAction SilentlyContinue|Select-Object LocalAddress,LocalPort,OwningProcess
    $cfg=$config.Content|ConvertFrom-Json;$idList=[string[]]($ids.Content|ConvertFrom-Json);$health=Invoke-OpencodeJson $runtime '/global/health'
    $override=$schemas.Content-match[regex]::Escape('Spike 001 fail-closed bash override');$listenerText=$listeners|Out-String
    $passed=$unauth.Status-eq401-and$doc.Status-eq200-and$health.healthy-and$cfg.permission.'*'-eq'deny'-and$override-and$listenerText-match'127\.0\.0\.1'-and$listenerText-notmatch'0\.0\.0\.0'
    $report=[pscustomobject]@{generated_at=(Get-Date).ToUniversalTime().ToString('o');opencode_version=$runtime.Version;executable_sha256=(Get-FileHash $c.OpencodeExecutable -Algorithm SHA256).Hash;base_url=$runtime.BaseUrl;pure_mode=$true;auto_mode=$false;unauthenticated_health_status=$unauth.Status;authenticated_health=$health;doc_status=$doc.Status;effective_permission=$cfg.permission;configured_model=$cfg.model;tool_ids=$idList;custom_bash_override_confirmed=$override;listeners=@($listeners);passed=$passed}
    New-Item -ItemType Directory -Path $c.EvidenceRoot -Force|Out-Null;[IO.File]::WriteAllText((Join-Path $c.EvidenceRoot 'spike-001-opencode-server-proof-result.json'),($report|ConvertTo-Json -Depth 15));$report|Format-List
    if(-not$passed){throw 'opencode server proof failed'};$completed=$true
}finally{if($runtime){Stop-OpencodeProofRuntime $runtime $completed 'opencode-failure-logs'}}
