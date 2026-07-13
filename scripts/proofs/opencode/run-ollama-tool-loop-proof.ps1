[CmdletBinding()] param([int]$Port=40962,[int]$RunCount=3)
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSScriptRoot '..\common\SpikeProofRuntime.psm1') -Force
$c=Get-SpikeProofConfig;$runtime=$null;$completed=$false
try{
    $runtime=Start-OpencodeProofRuntime -Name 'opencode-ollama-proof' -Port $Port -Permission @{'*'='deny';echo_value='allow'} -ToolFiles @((Join-Path $PSScriptRoot 'tools\bash.ts'),(Join-Path $PSScriptRoot 'tools\echo_value.ts'))
    Invoke-OpencodeJson $runtime "/experimental/tool?provider=$($c.ProviderId)&model=gemma4%3A12b" -TimeoutSec 60|Out-Null
    $runs=@();for($n=1;$n-le$RunCount;$n++){
        $session=Invoke-OpencodeJson $runtime '/session' Post @{title="Spike 001 Ollama tool-loop proof $n"}
        $response=Invoke-OpencodeJson $runtime "/session/$($session.id)/message" Post @{model=@{providerID=$c.ProviderId;modelID=$c.ModelId};parts=@(@{type='text';text='Call echo_value exactly once with value ready. After the tool result, answer with exactly DONE.'})} 180
        $messages=Invoke-OpencodeJson $runtime "/session/$($session.id)/message"
        $allParts=@(@($messages)|ForEach-Object{$_.parts});$echo=@($allParts|Where-Object{$_.type-eq'tool'-and$_.tool-eq'echo_value'});$final=(@($response.parts|Where-Object{$_.type-eq'text'}|ForEach-Object{$_.text})-join'')
        $runs+=[pscustomobject]@{run=$n;session_id=$session.id;echo_tool_call_count=$echo.Count;echo_input=@($echo|ForEach-Object{$_.state.input.value});echo_output=@($echo|ForEach-Object{$_.state.output});final_text=$final;bash_execution_absent=(($messages|ConvertTo-Json -Depth 30)-notmatch'blocked: arbitrary shell execution');passed=($echo.Count-eq1-and$final-eq'DONE')}
    }
    $passed=$runs.Count-eq$RunCount-and@($runs|Where-Object{-not$_.passed}).Count-eq0;$report=[pscustomobject]@{generated_at=(Get-Date).ToUniversalTime().ToString('o');opencode_version=$runtime.Version;ollama_base_url=$c.OllamaBaseUrl;model=$c.ModelId;run_count=$runs.Count;runs=$runs;passed=$passed}
    [IO.File]::WriteAllText((Join-Path $c.EvidenceRoot 'spike-001-opencode-ollama-proof-result.json'),($report|ConvertTo-Json -Depth 20));$runs|Format-Table run,session_id,echo_tool_call_count,final_text,bash_execution_absent,passed -AutoSize
    if(-not$passed){throw 'Ollama tool-loop proof failed'};$completed=$true
}finally{if($runtime){Stop-OpencodeProofRuntime $runtime $completed 'opencode-ollama-failure-logs'}}
