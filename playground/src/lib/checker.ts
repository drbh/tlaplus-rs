import type { Result } from './types';

export const browserChecker = import.meta.env.VITE_BROWSER_CHECKER === '1';

export async function check(request: unknown, signal: AbortSignal): Promise<Result> {
  if (!browserChecker) {
    const response = await fetch('/api/check', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(request), signal});
    return response.json();
  }
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./checker.worker.ts', import.meta.url), {type:'module'});
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener('abort', abort); worker.terminate(); };
    const abort = () => { cleanup(); reject(new DOMException('Check cancelled', 'AbortError')); };
    const timer = setTimeout(() => {
      cleanup();
      resolve({status:'incomplete', message:'15-second time limit reached. No verification verdict is available.'});
    }, 15_000);
    signal.addEventListener('abort', abort, {once:true});
    worker.onmessage = ({data}) => { cleanup(); resolve(data); };
    worker.onerror = event => { cleanup(); reject(new Error(event.message)); };
    worker.postMessage(request);
  });
}
