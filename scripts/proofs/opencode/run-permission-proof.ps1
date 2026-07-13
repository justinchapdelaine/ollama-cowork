[CmdletBinding()] param([int]$Port=40963)
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force
$c=Get-SpikeProofConfig;$runtime=$null;$completed=$false
try{
    Test-OllamaProofEndpoint -TimeoutSec 10|Out-Null
    $runtime=Start-OpencodeProofRuntime -Name 'opencode-permission-proof' -Port $Port -Permission @{'*'='deny';approval_probe='ask'} -ToolFiles @((Join-Path $PSScriptRoot 'tools\bash.ts'),(Join-Path $PSScriptRoot 'tools\approval_probe.ts'))
    Invoke-OpencodeJson $runtime "/experimental/tool?provider=$($c.ProviderId)&model=gemma4%3A12b" -TimeoutSec 60|Out-Null
    $driver=(Resolve-Path (Join-Path $PSScriptRoot 'permission-driver.mjs')).Path;$node=(Get-Command node.exe).Source;$results=@()
    foreach($action in 'once','reject','abort'){
        $result=$null
        foreach($attempt in 1..3){
            $session=Invoke-OpencodeJson $runtime '/session' Post @{title="Spike 001 permission proof $action attempt $attempt"}
            $token=('PERM_'+$action.ToUpperInvariant()+'_'+[guid]::NewGuid().ToString('N').Substring(0,12))
            $requestPath=Join-Path $runtime.Root "permission-$action-$attempt-request.json";$resultPath=Join-Path $runtime.Root "permission-$action-$attempt-result.json"
            $request=@{schema_version=1;base_url=$runtime.BaseUrl;authorization=$runtime.Authorization;session_id=$session.id;action=$action;token=$token;tool_name='approval_probe';argument_name='token';execution_marker='APPROVAL_PROBE_EXECUTED:';result_path=$resultPath;timeout_ms=60000}
            [IO.File]::WriteAllText($requestPath,($request|ConvertTo-Json -Depth 8))
            $previous=$ErrorActionPreference;try{$ErrorActionPreference='Continue';& $node $driver $requestPath|Out-Host;$exit=$LASTEXITCODE}finally{$ErrorActionPreference=$previous}
            if(-not(Test-Path $resultPath)){throw "permission driver did not write result for $action attempt $attempt"}
            $result=Get-Content -Raw $resultPath|ConvertFrom-Json
            if($result.passed){break}
            if($result.permission_requested){break}
        }
        $results+=$result
        if($exit-ne0-or-not$result.passed){[IO.File]::WriteAllText((Join-Path $c.EvidenceRoot "spike-001-opencode-permission-$action-diagnostic.json"),($result|ConvertTo-Json -Depth 20));throw "permission scenario failed: $action"}
    }
    $passed=$results.Count-eq3-and@($results|Where-Object{-not$_.passed}).Count-eq0
    $report=[pscustomobject]@{generated_at=(Get-Date).ToUniversalTime().ToString('o');opencode_version=$runtime.Version;model=$c.ModelId;permission=@{'*'='deny';approval_probe='ask'};tool='custom approval probe with explicit context.ask';scenarios=$results;passed=$passed}
    [IO.File]::WriteAllText((Join-Path $c.EvidenceRoot 'spike-001-opencode-permission-proof-result.json'),($report|ConvertTo-Json -Depth 20));$results|Format-Table action,permission_requested,response_event,abort_result,completed_tool_count,executed,passed -AutoSize
    if(-not$passed){throw 'permission proof failed'};$completed=$true
}finally{if($runtime){Stop-OpencodeProofRuntime $runtime $completed 'opencode-permission-failure-logs'}}
