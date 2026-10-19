import { WASI, File, OpenFile, PreopenDirectory, ConsoleStdout } from '@bjorn3/browser_wasi_shim';

self.onmessage = async ({data}) => {
  let stderr = "";
  try {
    const response = await fetch('/checker.wasm');
    if (!response.ok) throw new Error(`Could not load checker (${response.status})`);
    const module = await WebAssembly.compile(await response.arrayBuffer());
    const input = new File(new TextEncoder().encode(JSON.stringify(data)));
    const output = new File([]);
    const work = new PreopenDirectory('/work', new Map([['request.json', input], ['result.json', output]]));
    const wasi = new WASI(['tla-playground', '/work/request.json', '/work/result.json'], ['TMPDIR=/tmp'], [
      new OpenFile(new File([])), new OpenFile(new File([])),
      new ConsoleStdout(bytes => { stderr = (stderr + new TextDecoder().decode(bytes)).slice(-4000); }),
      new PreopenDirectory('/tmp', new Map()), work,
    ]);
    const instance = await WebAssembly.instantiate(module, {wasi_snapshot_preview1: wasi.wasiImport});
    const code = wasi.start(instance as Parameters<WASI['start']>[0]);
    if (code !== 0) throw new Error(stderr || `Checker exited with status ${code}`);
    if (output.data.length > 16_000_000) {
      self.postMessage({status:'incomplete', message:'Result exceeds the 16 MB display limit. Reduce the state limit.'});
    } else {
      self.postMessage(JSON.parse(new TextDecoder().decode(output.data)));
    }
  } catch (error) {
    self.postMessage({status:'error', message:`Checker could not finish: ${stderr || error}`});
  }
};
