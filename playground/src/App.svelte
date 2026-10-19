<script lang="ts">
  import {onMount} from 'svelte';
  import {check,browserChecker} from './lib/checker';
  import Editor from './components/Editor.svelte';
  import Inspector from './components/Inspector.svelte';
  import Graph from './components/Graph.svelte';
  import Trace from './components/Trace.svelte';
  import PuzzleView from './components/PuzzleView.svelte';
  import {examples,sourceFor} from './lib/examples';
  import type {Language,Result,Action} from './lib/types';
  let example=$state(0), language=$state<Language>('tsl');
  let source=$state(sourceFor(0,'tsl')), config=$state('');
  let result=$state.raw<Result|undefined>(), busy=$state(false), stale=$state(false);
  let tab=$state<'trace'|'graph'|'generated'>('trace'), selected=$state(0), step=$state(0);
  let comparedFrom=$state<number|undefined>();
  function selectGraph(id:number,from?:number){selected=id;comparedFrom=from}
  let limit=$state(500), copied=$state(false), left=$state(47), mobile=$state<'source'|'result'>('source');
  let editor:Editor, controller:AbortController|undefined, revision=0;
  let pending=$state(false),auto=$state(true),elapsed=$state(0),query=$state('');
  let timer:ReturnType<typeof setTimeout>|undefined, mounted=false;
  let dialog:HTMLDialogElement;
  let filtered=$derived(examples.map((item,index)=>({...item,index})).filter(item=>`${item.title} ${item.note}`.toLowerCase().includes(query.toLowerCase())));
  let filename=$derived((source.match(/^\s*(?:-+\s*MODULE|spec|module)\s+([A-Za-z0-9_]+)/m)?.[1]??'Model')+'.'+language);
  function schedule(){clearTimeout(timer);pending=false;if(!mounted||!auto)return;pending=true;timer=setTimeout(()=>{pending=false;void run()},800)}
  function openLibrary(){query='';dialog.showModal();dialog.querySelector('input')?.focus()}
  function closeLibrary(){dialog.close()}
  function chooseExample(index:number){load(index);closeLibrary()}
  function toggleAuto(){auto=!auto;if(auto)schedule();else stop()}

  let saved:Partial<Record<Language,{source:string;config:string}>>={};
  let current=$derived(result?.trace?.[step]);
  let node=$derived(result?.graph?.nodes.find(n=>n.id===selected));
  let parent=$derived(result?.graph?.nodes.find(n=>n.id===(comparedFrom??node?.parent)));
  let comparison=$derived(parent?`${comparedFrom!==undefined?'edge source':'discovery parent'} S${parent.id}`:'none');
  function persist(){try{localStorage.setItem('tla.playground.v1',JSON.stringify({source,config,language,example,limit}))}catch{}}
  function edit(text:string){source=text;changed()}
  function changed(){stop();stale=!!result;persist();schedule()}
  function stop(){clearTimeout(timer);pending=false;controller?.abort();controller=undefined;busy=false;revision++}
  async function run(showResults=false){
    stop();elapsed=0;const started=performance.now();const ticker=setInterval(()=>elapsed=(performance.now()-started)/1000,100);const runId=revision;const abort=new AbortController();controller=abort;busy=true;
    try{
      const next=await check({language,source,config,maxStates:limit},abort.signal);
      if(runId!==revision)return;
      result=next;stale=false;step=0;selected=0;comparedFrom=undefined;tab=next.trace?.length?'trace':'graph';if(showResults)mobile='result';
    }catch(e){if(!abort.signal.aborted&&runId===revision){result={status:'error',message:`Could not run checker: ${e}`};stale=false;}}
    finally{clearInterval(ticker);if(controller===abort){busy=false;controller=undefined}}
  }
  function load(index:number){stop();example=index;limit=examples[index].maxStates;saved={};source=sourceFor(index,language)||sourceFor(index,'tsl');if(!sourceFor(index,language))language='tsl';config='';result=undefined;stale=false;persist();schedule()}
  function switchLanguage(next:Language){
    if(next===language)return;stop();saved[language]={source,config};language=next;
    const draft=saved[next];source=draft?.source??sourceFor(example,next);config=draft?.config??'';
    if(!source){source='---- MODULE Model ----\nEXTENDS Integers\nVARIABLE x\nInit == x = 0\nNext == x\' = x\n====\n';config='INIT Init\nNEXT Next\n';}
    result=undefined;stale=false;persist();schedule();
  }
  function reveal(action:Action|null|undefined){if(action?.line&&!stale){mobile='source';editor.reveal(action.line,action.column??1)}}
  function openGenerated(){if(!result?.generated||stale)return;const generated=result.generated;stop();saved[language]={source,config};language='tla';source=generated.source;config=generated.config;result=undefined;persist();schedule()}
  async function share(){
    const payload=JSON.stringify({source,config,language,example,limit});
    const bytes=new TextEncoder().encode(payload);const encoded=btoa(Array.from(bytes,b=>String.fromCharCode(b)).join(''));
    if(encoded.length>100000){alert('This source is too large for a share link. Download the source instead.');return}
    const url=new URL(location.href);url.hash='model='+encoded;
    try{await navigator.clipboard.writeText(url.href);copied=true;setTimeout(()=>copied=false,1500)}catch{history.replaceState(null,'',url)}
  }
  function download(){
    const name=source.match(/^\s*(?:-+\s*MODULE|spec|module)\s+([A-Za-z0-9_]+)/m)?.[1]??'Model';
    const text=language==='tla'&&config.trim()?source.replace(/^-+\s*CONFIG\b[\s\S]*$/m,'')+`\n---- CONFIG ${name} ----\n${config}\n====\n`:source;
    const url=URL.createObjectURL(new Blob([text],{type:'text/plain'}));
    const a=document.createElement('a');a.href=url;a.download=`${name}.${language}`;a.click();URL.revokeObjectURL(url);
  }
  function resize(e:PointerEvent){const move=(e:PointerEvent)=>left=Math.max(25,Math.min(70,e.clientX/window.innerWidth*100));const up=()=>{window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',up)};window.addEventListener('pointermove',move);window.addEventListener('pointerup',up)}
  onMount(()=>{
    try{
      const hash=location.hash.startsWith('#model=')?location.hash.slice(7):null;
      const text=hash?new TextDecoder().decode(Uint8Array.from(atob(hash),c=>c.charCodeAt(0))):localStorage.getItem('tla.playground.v1');
      if(text){const s=JSON.parse(text);if(typeof s.source==='string'&&s.source.length<=256000&&(s.language==='tsl'||s.language==='tla')){source=s.source;config=typeof s.config==='string'?s.config:'';language=s.language;example=Number.isInteger(s.example)&&s.example>=0&&s.example<examples.length?s.example:0;limit=Math.max(1,Math.min(2000,Number(s.limit)||500));}}
    }catch{}
    mounted=true;schedule();
    return()=>{mounted=false;clearTimeout(timer);controller?.abort()};
  });
</script>
<svelte:window onkeydown={e=>{if((e.metaKey||e.ctrlKey)&&e.key==='k'){e.preventDefault();openLibrary()}}}/>
<svelte:head><title>TLA · Model playground</title></svelte:head>
<dialog aria-label="Example library" bind:this={dialog} onclick={e=>{if(e.target===dialog)closeLibrary()}} onkeydown={e=>{if(e.key==='Escape')closeLibrary()}}>
  <div class="library-head"><div><div class="eyebrow">EXAMPLE LIBRARY</div><h2>Find your next model</h2></div><button onclick={closeLibrary} aria-label="Close example library">✕</button></div>
  <input class="library-search" aria-label="Search examples" placeholder="Search puzzles, protocols, concepts…" bind:value={query}/>
  <div class="library-list">{#each filtered as item}<button onclick={()=>chooseExample(item.index)}><span class="library-icon">{item.puzzle==='paxos'?'⇄':item.puzzle?'◇':'λ'}</span><span><strong>{item.title}</strong><small>{item.note}</small></span><span class="library-arrow">↗</span></button>{/each}{#if !filtered.length}<p class="empty">No models match “{query}”. Try “Paxos” or “puzzle”.</p>{/if}</div>
  <div class="library-foot">{filtered.length} examples · editable TSL and TLA+ · Escape to close</div>
</dialog>
<div class="app">
  <header><span class="brand">tla</span><span class="sub">playground</span><div class="header-actions"><button class="browse" onclick={openLibrary}>Examples <kbd>⌘ K</kbd></button><button onclick={share}>{copied?'Copied':'Share'}</button><button onclick={download}>Download</button><span class="local">{browserChecker?'BROWSER CHECKER':'LOCAL CHECKER'}</span></div></header>
  <div class="mobile-tabs"><button class:active={mobile==='source'} onclick={()=>mobile='source'}>Source</button><button class:active={mobile==='result'} onclick={()=>mobile='result'}>Results {busy?'◌':result?'•':''}</button></div>
  <main style={`--left:${left}%`} class:show-result={mobile==='result'}>
    <section class="source-pane">
      <div class="toolbar"><select aria-label="Example" value={example} onchange={e=>load(Number(e.currentTarget.value))}>{#each examples as e,i}<option value={i}>{e.title}</option>{/each}</select><div class="language"><button class:active={language==='tsl'} onclick={()=>switchLanguage('tsl')}>TSL</button><button class:active={language==='tla'} onclick={()=>switchLanguage('tla')}>TLA+</button></div></div>
      <div class="filebar"><span><span class="file-icon">{language==='tsl'?'TS':'λ'}</span>{filename}</span><span class="draft-label">Local draft</span></div><p class="model-note">{examples[example].note}</p>
      <Editor bind:this={editor} {source} {language} onedit={edit} onrun={()=>run(true)} problem={!stale&&result?.status==='error'?result:undefined}/>
      {#if language==='tla'}<details class="config"><summary>TLC configuration <span>(optional when CONFIG is embedded)</span></summary><textarea aria-label="TLC configuration" bind:value={config} oninput={changed} placeholder="INIT Init&#10;NEXT Next&#10;INVARIANT TypeOK"></textarea></details>{/if}
      <div class="runbar"><button class="auto-toggle" class:active={auto} aria-pressed={auto} onclick={toggleAuto} title="Automatically check 800 ms after editing">{auto?'● Auto':'○ Paused'}</button><label>State limit <input aria-label="State limit" type="number" min="1" max="2000" bind:value={limit} oninput={changed}/></label><span>{pending?'Waiting for edits…':busy?`${elapsed.toFixed(1)}s`:'⌘ / Ctrl ↵'}</span>{#if busy}<button onclick={stop}>Stop</button>{/if}<button class="primary" onclick={()=>run(true)} disabled={busy}>{busy?'Checking…':'Run model →'}</button></div>
    </section>
    <button class="split" aria-label="Resize panes" onpointerdown={resize} onkeydown={e=>{if(e.key==='ArrowLeft')left=Math.max(25,left-2);if(e.key==='ArrowRight')left=Math.min(70,left+2)}}></button>
    <section class="results-pane" aria-label="Model results">
      <div class="toolbar"><div class="view-tabs"><button class:active={tab==='trace'} onclick={()=>tab='trace'}>Trace</button><button class:active={tab==='graph'} onclick={()=>tab='graph'}>State graph</button><button class:active={tab==='generated'} onclick={()=>tab='generated'}>Generated TLA+</button></div></div>
      {#if stale}<div class="notice">Source or settings changed. {pending||busy?'Updating results…':'Results below belong to the previous run.'}</div>{/if}
      {#if busy}<div class="notice" role="status">Checking your model… {elapsed.toFixed(1)}s <span>Exploring reachable states</span></div>{/if}
      {#if result}
        <div class={`verdict ${result.status}`} role="status"><div class="eyebrow">{result.status==='violated'?'COUNTEREXAMPLE':result.status.toUpperCase()}</div><h2>{result.message}</h2>{#if result.statistics}<div class="stats"><span><b>{result.statistics.states}</b> states retained</span><span><b>{result.statistics.initial}</b> initial</span><span><b>{result.statistics.depth}</b> reported search depth</span></div>{/if}{#if result.location}<button disabled={stale} onclick={()=>{mobile='source';editor.reveal(result!.location!.line,result!.location!.column)}}>Go to line {result.location.line}</button>{/if}</div>
        <div class="output">
          {#if result.properties?.length}<details class="property-results"><summary>Temporal checks · {result.properties.filter(p=>p.holds).length}/{result.properties.length} hold</summary><table aria-label="Temporal checks"><tbody>{#each result.properties as property}<tr><th>{property.name}</th><td class:property-failed={!property.holds}>{property.holds?'holds':'violated'}</td></tr>{/each}</tbody></table></details>{/if}
          {#if tab==='trace'}
            {#if result.trace?.length}
              <Trace trace={result.trace} loopBack={result.loopBack} graph={result.graph} selected={step} onselect={i=>step=i} onreveal={reveal} ongraph={id=>{selectGraph(id);tab='graph'}} {stale}/>
              {#if current}<Inspector values={current.values} previous={step>0?result.trace[step-1].values:[]} comparison={`trace step ${step-1}`}/><PuzzleView kind={examples[example].puzzle} values={current.values}/>{/if}
            {:else}<div class="empty"><h3>No counterexample trace</h3><p>{result.status==='passed'?'Explore the state graph to see all checked behavior.':'This result has no finite state trace to display.'}</p></div>{/if}
          {:else if tab==='graph'}
            {#if result.graph}<Graph graph={result.graph} {selected} onselect={selectGraph} onreveal={reveal} {stale}/>{#if node}<div class="trace-heading"><h3>State S{node.id}</h3><span>{node.expanded?'Successors explored':'Not fully expanded'}</span></div><Inspector values={node.values} previous={parent?.values??[]} {comparison}/><PuzzleView kind={examples[example].puzzle} values={node.values}/>{/if}{:else}<div class="empty">No graph is available for this result.</div>{/if}
          {:else}
            {#if result.generated}<div class="trace-heading"><h3>Compiled model</h3><button disabled={stale} onclick={openGenerated}>Open as editable TLA+ ↗</button></div><div class="generated-editor"><Editor source={result.generated.source} language="tla" readonly/></div><h3>TLC configuration</h3><pre>{result.generated.config}</pre>{:else}<div class="empty">Run a TSL model to inspect its generated TLA+ and configuration. Direct TLA+ runs use your source as written.</div>{/if}
          {/if}
        </div>
      {:else}<div class="welcome"><div class="eyebrow">YOUR MODEL, EXPLAINED</div><h1>A little code.<br/>Every possible path.</h1><p>Edit on the left. After you pause, the checker explores your model and shows what can happen.</p><div class="preview-path" aria-hidden="true"><span>S₀</span><i>→</i><span>S₁</span><i>→</i><span class="target">S₂</span></div><div class="welcome-cards"><div><strong>Follow a trace</strong><span>See the exact steps behind a failed property—or a puzzle solution.</span></div><div><strong>Explore the graph</strong><span>Select a reachable state and inspect the actions that connect it.</span></div></div><p class="welcome-foot">{pending?'Starting automatically…':busy?'Exploring your model…':'Enable Auto or run your model to begin.'}</p></div>{/if}
    </section>
  </main>
  <footer><span>Finite-state model checking · {browserChecker?'runs in your browser':'native engine'}</span><span>15 s / 2,000 states max · results are not an unbounded proof</span></footer>
</div>
