<script lang="ts">
  import type {Result} from '../lib/types';
  let {graph,selected,onselect}: {graph:NonNullable<Result['graph']>;selected:number;onselect:(id:number)=>void}=$props();
  let zoom=$state(1);
  let nodes=$derived(graph.nodes.slice(0,250));
  let positions=$derived.by(()=>{
    const levels=new Map<number,number>();
    return new Map(nodes.map(n=>{const row=levels.get(n.depth)??0;levels.set(n.depth,row+1);return[n.id,{x:65+n.depth*145,y:65+row*90}]}));
  });
  let width=$derived(Math.max(550,...[...positions.values()].map(p=>p.x+90)));
  let height=$derived(Math.max(230,...[...positions.values()].map(p=>p.y+65)));
  let edges=$derived(graph.edges.filter(e=>positions.has(e.from)&&positions.has(e.to)).slice(0,1200));
  function path(from:number,to:number){const a=positions.get(from)!,b=positions.get(to)!;
    if(from===to)return `M ${a.x-12} ${a.y-20} C ${a.x-65} ${a.y-80}, ${a.x+65} ${a.y-80}, ${a.x+12} ${a.y-20}`;
    if(a.x===b.x)return `M ${a.x+24} ${a.y} Q ${a.x+75} ${(a.y+b.y)/2} ${b.x+24} ${b.y}`;
    const dx=b.x>a.x?26:-26;return `M ${a.x+dx} ${a.y} Q ${(a.x+b.x)/2} ${(a.y+b.y)/2-18} ${b.x-dx} ${b.y}`;
  }
</script>
<div class="graph-tools"><span>{graph.complete?'Complete explored graph':'Partial explored graph'}{graph.quotient?' · symmetry quotient':''}</span><label>Zoom <input aria-label="Graph zoom" type="range" min="0.4" max="1.5" step="0.1" bind:value={zoom}/></label></div>
{#if graph.nodes.length>250||graph.edges.length>1200}<p class="notice">Display limited to 250 states / 1,200 edges. This is a display limit, separate from the checker’s verdict.</p>{/if}
<div class="graph-scroll">
  <svg width={width*zoom} height={height*zoom} viewBox={`0 0 ${width} ${height}`} aria-label="Explored state transitions">
    <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="var(--faint)"/></marker></defs>
    {#each edges as edge}
      <path d={path(edge.from,edge.to)} fill="none" stroke={edge.from===selected||edge.to===selected?'var(--accent)':'var(--line)'} stroke-width={edge.from===selected||edge.to===selected?2:1} marker-end="url(#arrow)"><title>S{edge.from} → S{edge.to}: {edge.action.name}</title></path>
    {/each}
    {#each nodes as node}
      {@const p=positions.get(node.id)!}
      <g role="button" tabindex="0" aria-label={`Select state ${node.id}`} onclick={()=>onselect(node.id)} onkeydown={e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();onselect(node.id)}}}>
        <circle cx={p.x} cy={p.y} r="25" fill={node.id===selected?'var(--accent-bg)':'var(--panel)'} stroke={node.id===selected?'var(--accent)':node.parent===null?'var(--accent)':'var(--faint)'} stroke-width="2" stroke-dasharray={node.expanded?'':'4 3'}/>
        <text x={p.x} y={p.y+4} text-anchor="middle" fill="var(--ink)" font-size="12">S{node.id}</text>
        <title>{node.values.map(v=>`${v.name} = ${v.value.text}`).join('\n')}{node.expanded?'':'\nSuccessors not fully explored'}</title>
      </g>
    {/each}
  </svg>
</div>
<p class="legend">Blue outline: initial · dashed: not fully expanded · arrows: observed actions. Select a state to inspect its transitions below.</p>
