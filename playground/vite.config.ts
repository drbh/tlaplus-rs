// Local-only adapter. Every run gets a killable process and a fresh directory.
import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import type { IncomingMessage, ServerResponse } from 'node:http';
const binary = resolve(import.meta.dirname, '../target/debug/tla-playground');
let active = 0;
async function run(req: IncomingMessage, res: ServerResponse) {
  const send = (status: number, message: unknown) => {
    if (!res.destroyed) { res.writeHead(status, { 'Content-Type':'application/json', 'Cache-Control':'no-store' }); res.end(JSON.stringify(message)); }
  };
  if (req.method !== 'POST' || !req.headers['content-type']?.startsWith('application/json')) return send(405, {status:'error',message:'POST JSON required'});
  if (req.headers.origin && req.headers.origin !== `http://${req.headers.host}`) return send(403, {status:'error',message:'Same-origin requests only'});
  if (active >= 2) return send(429, {status:'error',message:'Two checks are already running. Try again shortly.'});
  active++;
  let directory: string | undefined;
  try {
    const chunks: Buffer[] = []; let length = 0;
    for await (const chunk of req) {
      length += chunk.length;
      if (length > 300_000) { send(413,{status:'error',message:'Source exceeds request limit'}); return; }
      chunks.push(chunk);
    }
    JSON.parse(Buffer.concat(chunks).toString());
    directory = await mkdtemp(resolve(tmpdir(), 'tla-web-'));
    const input = resolve(directory, 'request.json'), output = resolve(directory,'result.json');
    await writeFile(input, Buffer.concat(chunks));
    const child = spawn(binary, [input,output], {cwd:directory, stdio:['ignore','ignore','pipe']});
    let stderr = '', expired = false;
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-4000); });
    const timer = setTimeout(() => { expired = true; child.kill('SIGKILL'); }, 15_000);
    const cancel = () => child.kill('SIGKILL');
    res.on('close',cancel);
    try {
      await new Promise<void>((accept,reject) => { child.on('error',reject); child.on('close',() => accept()); });
      if (res.destroyed) return;
      if (expired) return send(200,{status:'incomplete',message:'15-second time limit reached. No verification verdict is available.'});
      const bytes = await readFile(output);
      if (bytes.length > 16_000_000) return send(200,{status:'incomplete',message:'Result exceeds the 16 MB display limit. Reduce the state limit.'});
      send(200, JSON.parse(bytes.toString()));
    } catch (error) {
      send(500,{status:'error',message:`Checker could not finish. ${stderr || String(error)}`});
    } finally { clearTimeout(timer); res.off('close',cancel); }
  } catch (error) { send(400,{status:'error',message:String(error)}); }
  finally { active--; if(directory) await rm(directory,{recursive:true,force:true}); }
}
function checker(): Plugin {
  const install = (server: {middlewares: {use: (path:string, handler: typeof run)=>void}}) => { server.middlewares.use('/api/check', run); };
  return {name:'local-checker', configureServer:install, configurePreviewServer:install};
}
export default defineConfig({plugins:[svelte(),checker()], server:{port:7821,strictPort:true},preview:{port:7821,strictPort:true}});
