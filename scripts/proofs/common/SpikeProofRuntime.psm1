Set-StrictMode -Version Latest

function Get-SpikeProofConfig {
    $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
    $ollamaOrigin = $env:OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN
    if ([string]::IsNullOrWhiteSpace($ollamaOrigin)) {
        throw 'Set OLLAMA_COWORK_PROOF_OLLAMA_ORIGIN to the explicit Ollama host before running remote proofs.'
    }
    $ollamaOrigin = $ollamaOrigin.TrimEnd('/')
    $modelId = if ([string]::IsNullOrWhiteSpace($env:OLLAMA_COWORK_PROOF_OLLAMA_MODEL)) { 'gemma4:12b' } else { $env:OLLAMA_COWORK_PROOF_OLLAMA_MODEL }
    [pscustomobject]@{
        RepoRoot = $repoRoot
        EvidenceRoot = Join-Path $repoRoot 'docs\test-plans'
        CargoTargetRoot = Join-Path $repoRoot 'target\debug'
        DocxToolExecutable = Join-Path $repoRoot 'target\debug\ollama-cowork-docx-tool.exe'
        BrokerHostExecutable = Join-Path $repoRoot 'target\debug\ollama-cowork-broker-host.exe'
        BrokerProofExecutable = Join-Path $repoRoot 'target\debug\ollama-cowork-broker-proof.exe'
        OpencodeVersion = '1.17.18'
        OpencodeExecutable = Join-Path $env:APPDATA 'npm\node_modules\opencode-ai\bin\opencode.exe'
        ProviderId = 'ollama-lan'
        ProviderName = 'Ollama (LAN)'
        OllamaOrigin = $ollamaOrigin
        OllamaBaseUrl = $ollamaOrigin + '/v1'
        ModelId = $modelId
        SrtVersion = '0.0.65'
        SrtWinPath = 'C:\Program Files\ollama-cowork-spike\srt\0.0.65\srt-win.exe'
    }
}

function New-SpikeInlineConfig {
    param([hashtable]$Permission)
    $c = Get-SpikeProofConfig
    @{
        '$schema' = 'https://opencode.ai/config.json'
        model = "$($c.ProviderId)/$($c.ModelId)"
        permission = $Permission
        provider = @{
            $c.ProviderId = @{
                npm = '@ai-sdk/openai-compatible'
                name = $c.ProviderName
                options = @{ baseURL = $c.OllamaBaseUrl }
                models = @{ $c.ModelId = @{ name = $c.ModelId } }
            }
        }
    }
}

function New-RandomSecret {
    $bytes = [byte[]]::new(32)
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    [Convert]::ToBase64String($bytes)
}

function Invoke-OpencodeWebRequest {
    param($Runtime,[string]$Path,[bool]$Authenticated=$true,[int]$TimeoutSec=10)
    $headers = @{}
    if ($Authenticated) { $headers.Authorization = $Runtime.Authorization }
    try {
        $r = Invoke-WebRequest -UseBasicParsing -Uri ($Runtime.BaseUrl + $Path) -Headers $headers -TimeoutSec $TimeoutSec
        [pscustomobject]@{ Status=[int]$r.StatusCode; Content=$r.Content }
    } catch {
        if ($_.Exception.Response) { return [pscustomobject]@{ Status=[int]$_.Exception.Response.StatusCode; Content='' } }
        throw "API request $Path failed: $($_.Exception.Message)"
    }
}

function Invoke-OpencodeJson {
    param($Runtime,[string]$Path,[string]$Method='Get',$Body=$null,[int]$TimeoutSec=30)
    $p = @{ Uri=$Runtime.BaseUrl+$Path; Headers=@{Authorization=$Runtime.Authorization}; Method=$Method; TimeoutSec=$TimeoutSec }
    if ($null -ne $Body) { $p.ContentType='application/json'; $p.Body=($Body|ConvertTo-Json -Depth 20 -Compress) }
    Invoke-RestMethod @p
}

function Test-OllamaProofEndpoint {
    param([int]$TimeoutSec=10)
    $c=Get-SpikeProofConfig
    try {
        $version=Invoke-RestMethod -Uri ($c.OllamaOrigin+'/api/version') -TimeoutSec $TimeoutSec
        $models=Invoke-RestMethod -Uri ($c.OllamaOrigin+'/v1/models') -TimeoutSec $TimeoutSec
    } catch {
        throw "Configured Ollama endpoint $($c.OllamaOrigin) is unreachable; verify the host and LAN route, then retry. $($_.Exception.Message)"
    }
    $ids=@($models.data|ForEach-Object{$_.id})
    if($ids-notcontains$c.ModelId){throw "Configured Ollama endpoint is reachable, but exact model $($c.ModelId) is absent from /v1/models"}
    [pscustomobject]@{Origin=$c.OllamaOrigin;Version=$version.version;Model=$c.ModelId;Reachable=$true}
}

function Start-OpencodeProofRuntime {
    param([string]$Name,[int]$Port,[hashtable]$Permission,[string[]]$ToolFiles,[string]$LogLevel='INFO',[hashtable]$Environment=@{})
    $c=Get-SpikeProofConfig
    if(-not(Test-Path -LiteralPath $c.OpencodeExecutable)){throw "opencode missing: $($c.OpencodeExecutable)"}
    $v=(& $c.OpencodeExecutable --version).Trim(); if($v-ne$c.OpencodeVersion){throw "Expected $($c.OpencodeVersion), found $v"}
    $temp=[IO.Path]::GetFullPath($env:TEMP).TrimEnd('\'); $root=Join-Path $temp ("ollama-cowork-$Name-"+[guid]::NewGuid().ToString('N'))
    $workspace=Join-Path $root 'workspace'; $tools=Join-Path $workspace '.opencode\tools'; $xdg=Join-Path $root 'config-home'; $app=Join-Path $root 'appdata'
    New-Item -ItemType Directory -Path $tools,$xdg,$app -Force|Out-Null
    foreach($f in $ToolFiles){Copy-Item -LiteralPath $f -Destination $tools}
    $secret=New-RandomSecret; $auth='Basic '+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("opencode:$secret"))
    $old=@{Password=$env:OPENCODE_SERVER_PASSWORD;Config=$env:OPENCODE_CONFIG_CONTENT;Xdg=$env:XDG_CONFIG_HOME;App=$env:APPDATA}
    $environmentBefore=@{};foreach($key in $Environment.Keys){$environmentBefore[$key]=[Environment]::GetEnvironmentVariable($key,'Process')}
    try{
        $env:OPENCODE_SERVER_PASSWORD=$secret; $env:OPENCODE_CONFIG_CONTENT=(New-SpikeInlineConfig $Permission|ConvertTo-Json -Depth 20 -Compress); $env:XDG_CONFIG_HOME=$xdg; $env:APPDATA=$app
        foreach($key in $Environment.Keys){[Environment]::SetEnvironmentVariable($key,[string]$Environment[$key],'Process')}
        $proc=Start-Process -FilePath $c.OpencodeExecutable -ArgumentList @('serve','--pure','--hostname','127.0.0.1','--port',"$Port",'--print-logs','--log-level',$LogLevel) -WorkingDirectory $workspace -WindowStyle Hidden -RedirectStandardOutput (Join-Path $root 'stdout.log') -RedirectStandardError (Join-Path $root 'stderr.log') -PassThru
    } finally { $env:OPENCODE_SERVER_PASSWORD=$old.Password;$env:OPENCODE_CONFIG_CONTENT=$old.Config;$env:XDG_CONFIG_HOME=$old.Xdg;$env:APPDATA=$old.App;foreach($key in $Environment.Keys){[Environment]::SetEnvironmentVariable($key,$environmentBefore[$key],'Process')} }
    $runtime=[pscustomobject]@{Name=$Name;Root=$root;TempRoot=$temp;Workspace=$workspace;Process=$proc;BaseUrl="http://127.0.0.1:$Port";Authorization=$auth;Version=$v;Port=$Port;Config=$c}
    for($i=0;$i-lt 60;$i++){if($proc.HasExited){throw "opencode exited: $($proc.ExitCode)"};try{$h=Invoke-OpencodeJson $runtime '/global/health' -TimeoutSec 5;if($h.healthy){return $runtime}}catch{};Start-Sleep -Milliseconds 250}
    throw 'opencode health did not become ready'
}

function Stop-OpencodeProofRuntime {
    param($Runtime,[bool]$Completed,[string]$FailureLogName)
    if($Runtime.Process -and -not $Runtime.Process.HasExited){Stop-Process -Id $Runtime.Process.Id -Force -ErrorAction SilentlyContinue;$Runtime.Process.WaitForExit(5000)|Out-Null}
    if(-not $Completed -and $FailureLogName){$d=Join-Path $Runtime.Config.EvidenceRoot $FailureLogName;New-Item -ItemType Directory -Path $d -Force|Out-Null;foreach($n in 'stdout.log','stderr.log'){$p=Join-Path $Runtime.Root $n;if(Test-Path $p){Copy-Item $p (Join-Path $d $n) -Force}}}
    $resolved=[IO.Path]::GetFullPath($Runtime.Root);if($resolved.StartsWith($Runtime.TempRoot+'\ollama-cowork-',[StringComparison]::OrdinalIgnoreCase)){Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue}else{throw "Unsafe cleanup path: $resolved"}
}

Export-ModuleMember -Function Get-SpikeProofConfig,New-SpikeInlineConfig,Start-OpencodeProofRuntime,Stop-OpencodeProofRuntime,Invoke-OpencodeWebRequest,Invoke-OpencodeJson,Test-OllamaProofEndpoint
