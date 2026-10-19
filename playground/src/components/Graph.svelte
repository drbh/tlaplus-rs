<script lang="ts">
  import type {Result,Action} from '../lib/types';
  import {stateRows} from '../lib/state-view';
  import {rankLayout,neighborhood} from '../lib/graph-layout';
  let {graph,selected,onselect,onreveal,stale=false}: {graph:NonNullable<Result['graph']>;selected:number;onselect:(id:number,from?:number)=>void;onreveal?:(action:Action)=>void;stale?:boolean}=$props();
  let valueField=$state<string|null>(null);
  let fields=$derived([...new Set(graph.nodes.flatMap(node=>stateRows(node.values).map(row=>row.path)))]);
  let label=$derived(valueField??fields[0]??'');
  let scope=$state('auto'),zoom=$state(1),jump=$state<number|undefined>(undefined),error=$state(''),direction=$state('out'),page=$state(0);
  let focused=$derived(scope==='neighbors'||scope==='auto'&&graph.nodes.length>80);
  let selectedIndex=$derived(Math.max(0,graph.nodes.findIndex(node=>node.id===selected)));
  let start=$derived(Math.floor(selectedIndex/250)*250);
  let nodes=$derived(focused?neighborhood(graph.nodes,graph.edges,selected):graph.nodes.slice(start,start+250));
  let nodeValues=$derived(new Map(nodes.map(node=>[node.id,stateRows(node.values).find(row=>row.path===label)?.value??'—'])));
  let layout=$derived(rankLayout(nodes));
  let matching=$derived(graph.edges.filter(e=>layout.positions.has(e.from)&&layout.positions.has(e.to)));
  let edges=$derived([...matching].sort((a,b)=>Number(b.from===selected||b.to===selected)-Number(a.from===selected||a.to===selected)).slice(0,1200));
  let incoming=$derived(graph.edges.filter(e=>e.to===selected));
  let outgoing=$derived(graph.edges.filter(e=>e.from===selected));
  let transitions=$derived(direction==='out'?outgoing:incoming);
  $effect(()=>{selected;direction;page=0});
  function find(){if(jump!==undefined&&graph.nodes.some(n=>n.id===jump)){error='';onselect(jump)}else error='No state with that ID in this run.'}
  function path(from:number,to:number){const a=layout.positions.get(from)!,b=layout.positions.get(to)!;
    if(from===to)return `M ${a.x+30} ${a.y-5} C ${a.x+49} ${a.y-28}, ${a.x+49} ${a.y+28}, ${a.x+30} ${a.y+5}`;
    if(a.y===b.y)return `M ${a.x} ${a.y-14} Q ${(a.x+b.x)/2} ${a.y-37} ${b.x} ${b.y-14}`;
    if(b.y<a.y)return `M ${a.x-30} ${a.y} C ${Math.min(a.x,b.x)-40} ${a.y}, ${Math.min(a.x,b.x)-40} ${b.y}, ${b.x-30} ${b.y}`;
    return `M ${a.x} ${a.y+14} C ${a.x} ${(a.y+b.y)/2}, ${b.x} ${(a.y+b.y)/2}, ${b.x} ${b.y-14}`;
  }
</script>
<div class="graph-tools"><span>{graph.complete?'Complete explored graph':'Partial explored graph'}{graph.quotient?' · symmetry quotient':''}</span><select aria-label="Graph scope" value={focused?'neighbors':'overview'} onchange={e=>scope=e.currentTarget.value}><option value="overview">Overview</option><option value="neighbors">Selected neighborhood</option></select><label>Zoom <input aria-label="Graph zoom" type="range" min="0.5" max="1.5" step="0.1" bind:value={zoom}/></label></div>
<div class="graph-labels"><label>State value <select aria-label="Graph state value" value={label} onchange={e=>valueField=e.currentTarget.value}><option value="">IDs only</option>{#each fields as field}<option value={field}>{field}</option>{/each}</select></label></div>
<div class="graph-navigation"><form onsubmit={e=>{e.preventDefault();find()}}><label>Find S<input aria-label="Find state ID" type="number" min="0" bind:value={jump} placeholder={String(selected)}/></label><button>Go</button></form><span>{nodes.length}/{graph.nodes.length} states · {edges.length}/{graph.edges.length} edges shown</span>{#if !focused&&graph.nodes.length>250}<button disabled={start===0} onclick={()=>onselect(graph.nodes[start-250].id)}>← 250</button><button disabled={start+250>=graph.nodes.length} onclick={()=>onselect(graph.nodes[start+250].id)}>250 →</button>{/if}</div>
{#if error}<p class="notice">{error}</p>{/if}
{#if nodes.length<graph.nodes.length||edges.length<graph.edges.length}<p class="graph-scope-note">{focused?'One-hop neighborhood; up to 80 states.':'Overview window; up to 250 states.'} Only edges between displayed states are drawn (up to 1,200). All incident transitions remain available below.</p>{/if}
<div class="graph-scroll">
  <svg width={layout.width*zoom} height={layout.height*zoom} viewBox={`0 0 ${layout.width} ${layout.height}`} aria-label="Explored state transitions">
    <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke"/></marker></defs>
    {#each layout.ranks as rank}<text x="5" y={rank.y+3} class="rank-label">d{rank.depth}</text>{/each}
    {#each edges as edge}
      {@const active=edge.from===selected||edge.to===selected}
      {@const back=layout.positions.get(edge.to)!.y<=layout.positions.get(edge.from)!.y}
      <path d={path(edge.from,edge.to)} fill="none" stroke={active?'var(--accent)':back?'var(--warn)':'var(--faint)'} opacity={active?1:.4} stroke-width={active?1.5:1} marker-end="url(#arrow)"><title>S{edge.from} → S{edge.to}: {edge.action.name}</title></path>
    {/each}
    {#each nodes as node}
      {@const p=layout.positions.get(node.id)!}
      <g role="button" tabindex="0" aria-label={`Select state ${node.id}`} aria-pressed={node.id===selected} onclick={()=>onselect(node.id)} onkeydown={e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onselect(node.id)}}}>
        <rect x={p.x-30} y={p.y-14} width="60" height="28" rx="3" fill={node.id===selected?'var(--accent-bg)':'var(--panel)'} stroke={node.id===selected||node.parent===null?'var(--accent)':'var(--faint)'} stroke-width={node.id===selected?2:1} stroke-dasharray={node.expanded?'':'3 2'}/>
        <text x={p.x} y={label?p.y-2:p.y+3} text-anchor="middle" fill="var(--ink)" font-size="10">S{node.id}{node.parent===null?'*':''}</text>
        {#if label}<text x={p.x} y={p.y+9} text-anchor="middle" fill="var(--muted)" font-size="8">{(nodeValues.get(node.id)??'—').length>9?(nodeValues.get(node.id)??'').slice(0,8)+'…':nodeValues.get(node.id)}</text>{/if}
        <title>{node.values.map(v=>`${v.name} = ${v.value.text}`).join('\n')}{node.expanded?'':'\nSuccessors not fully explored'}</title>
      </g>
    {/each}
  </svg>
</div>
<p class="legend">d = discovery depth · * initial · dashed = unexpanded · amber = back / same-rank edges · arrows preserve merges and loops.</p>
<div class="edge-head"><strong>S{selected}</strong><button class:active={direction==='out'} onclick={()=>direction='out'}>Outgoing {outgoing.length}</button><button class:active={direction==='in'} onclick={()=>direction='in'}>Incoming {incoming.length}</button><span>Follow a destination to compare that edge.</span></div>
<div class="edge-list"><table aria-label="State transitions"><thead><tr><th>From</th><th>Action</th><th>To</th></tr></thead><tbody>{#each transitions.slice(page*100,(page+1)*100) as edge}<tr><td><button onclick={()=>onselect(edge.from)}>S{edge.from}</button></td><td>{#if edge.action.line&&onreveal}<button class="edge-action" disabled={stale} onclick={()=>onreveal?.(edge.action)} title="Show action in source">{edge.action.name} ↗</button>{:else}{edge.action.name}{/if}</td><td><button onclick={()=>onselect(edge.to,edge.from)} title={`Inspect S${edge.to} compared with S${edge.from}`}>→ S{edge.to}{edge.from===edge.to?' ↶':''}</button></td></tr>{/each}</tbody></table>{#if !transitions.length}<p class="inspector-empty">No {direction==='out'?'outgoing':'incoming'} transitions recorded.{direction==='out'&&!graph.nodes.find(n=>n.id===selected)?.expanded?' This state was not fully expanded.':''}</p>{/if}</div>
{#if transitions.length>100}<div class="edge-head"><button disabled={page===0} onclick={()=>page--}>Previous</button><span>{page*100+1}–{Math.min((page+1)*100,transitions.length)} of {transitions.length}</span><button disabled={(page+1)*100>=transitions.length} onclick={()=>page++}>Next</button></div>{/if}
