import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { SandboxManager } from "@anthropic-ai/sandbox-runtime";

const request=JSON.parse(await readFile(process.argv[2],"utf8"));
if(request.schema_version!==1)throw new Error("unsupported bridge schema");
const hash=async p=>createHash("sha256").update(await readFile(p)).digest("hex");
const sourceBefore=await hash(request.source);let resetError=null,tool=null,error=null;
const toolRequest={schema_version:1,input:request.source,operation:request.operation.operation};
if(request.operation.operation==="rewrite_section"){toolRequest.output=request.output;toolRequest.heading=request.operation.heading;toolRequest.replacement_paragraphs=request.operation.replacement_paragraphs;}
const toolRequestPath=request.result_path.replace(/\.json$/,"-tool-request.json");await writeFile(toolRequestPath,JSON.stringify(toolRequest));
const settings={network:{allowedDomains:[],deniedDomains:[],allowLocalBinding:false},filesystem:{denyRead:[],allowRead:[...request.read_roots,request.source,request.output.substring(0,request.output.lastIndexOf("\\"))],allowWrite:[request.output.substring(0,request.output.lastIndexOf("\\"))],denyWrite:[request.source]},windows:{srtWin:{path:request.srt_win}}};
try{await SandboxManager.initialize(settings);const command=`"${request.docx_tool}" --request "${toolRequestPath}"`;const wrapped=await SandboxManager.wrapWithSandboxArgv(command,undefined,undefined,undefined,request.read_roots[0]);tool=await new Promise((resolve,reject)=>{const child=spawn(wrapped.argv[0],wrapped.argv.slice(1),{cwd:request.read_roots[0],env:wrapped.env,windowsHide:true,stdio:["ignore","pipe","pipe"]});let stdout="",stderr="";const timer=setTimeout(()=>{child.kill();reject(new Error("DOCX tool timeout"));},request.timeout_ms);child.stdout.on("data",v=>{stdout+=v;if(stdout.length>request.stdout_limit)child.kill();});child.stderr.on("data",v=>stderr+=v);child.once("error",reject);child.once("exit",code=>{clearTimeout(timer);resolve({code,stdout,stderr});});});if(tool.code!==0)error=`DOCX tool failed: ${tool.stdout||tool.stderr}`;}catch(e){error=String(e);}finally{try{await SandboxManager.reset();}catch(e){resetError=String(e);}}
const sourceAfter=await hash(request.source);let parsed=null;try{parsed=JSON.parse(tool?.stdout||"null");}catch{}const passed=!error&&!resetError&&sourceBefore===sourceAfter&&parsed?.status&&(parsed.status==="rewritten"||parsed.status==="inspected"||parsed.status==="valid");const result={passed,source_sha256_after:sourceAfter,private_artifact:parsed?.output??null,tool_result:tool?.stdout?.trim()??"",error:error||resetError||(sourceBefore!==sourceAfter?"source changed":null)};await writeFile(request.result_path,JSON.stringify(result,null,2));process.exitCode=passed?0:1;
