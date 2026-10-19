<script lang="ts">
  import type {Result,Action} from '../lib/types';
  import {stateRows,stateKey,graphStateIndex} from '../lib/state-view';
  let {trace,loopBack,graph,selected,onselect,onreveal,ongraph,stale=false}: {
    trace:NonNullable<Result['trace']>;loopBack:Result['loopBack'];graph:Result['graph'];selected:number;
    onselect:(index:number)=>void;onreveal:(action:Action)=>void;ongraph:(id:number)=>void;stale?:boolean;
  }=$props();
  let table:HTMLTableElement;
  $effect(()=>{trace;choices=null});
  $effect(()=>{const index=selected;table?.querySelectorAll('.trace-select')[index]?.scrollIntoView({block:'nearest',inline:'nearest'})});
  let choices=$state<string[]|null>(null);
  let rows=$derived(trace.map(step=>new Map(stateRows(step.values).map(row=>[row.path,row.value]))));
  let fields=$derived([...new Set(rows.flatMap(row=>[...row.keys()]))]);
  let varying=$derived(fields.filter(field=>new Set(rows.map(row=>row.get(field))).size>1));
  let columns=$derived((choices??[...varying,...fields.filter(f=>!varying.includes(f))].slice(0,4)).filter(f=>fields.includes(f)));
  let states=$derived(graphStateIndex(graph?.nodes??[]));
  let selectedGraphId=$derived(states.get(stateKey(trace[selected].values)));
  function keyboard(e:KeyboardEvent,index:number){
    if(e.key!=='ArrowDown'&&e.key!=='ArrowUp')return;
    e.preventDefault();const next=Math.max(0,Math.min(trace.length-1,index+(e.key==='ArrowDown'?1:-1)));
    select(next);table.querySelectorAll<HTMLButtonElement>('.trace-select')[next]?.focus();
  }
  function toggle(field:string){const next=new Set(columns);if(next.has(field))next.delete(field);else next.add(field);choices=[...next]}
  function changeCount(index:number){if(!index)return 0;return fields.filter(f=>rows[index].get(f)!==rows[index-1].get(f)).length}
  function select(index:number){onselect(Math.max(0,Math.min(trace.length-1,index)))}
</script>
<div class="trace-summary">
  <strong>Trace</strong><span>{trace.length-1} transitions{loopBack?' + loop':''}</span><span>· {trace.length} states{loopBack?' · repeating behavior':''}</span>
  <details class="trace-columns"><summary>Columns {columns.length}/{fields.length}</summary><div>{#each fields as field}<label><input type="checkbox" checked={columns.includes(field)} onchange={()=>toggle(field)}/>{field}</label>{/each}</div></details>
</div>
<div class="trace-table-scroll">
  <table bind:this={table} class="trace-table" aria-label="Counterexample trace">
    <thead><tr><th scope="col">Step</th><th scope="col">Action / changes</th>{#each columns as field}<th scope="col" title={field}>{field}</th>{/each}</tr></thead>
    <tbody>{#each trace as step,i}
      {@const changes=changeCount(i)}
      <tr class:selected={selected===i} class:loop-target={loopBack?.index===i}>
        <th scope="row" class="trace-index"><button class="trace-select" aria-label={`Select trace step ${i}`} aria-current={selected===i?'step':undefined} onclick={()=>select(i)} onkeydown={e=>keyboard(e,i)}><span class="trace-rail" class:first={i===0} class:last={i===trace.length-1}>●</span><span>{i}</span>{#if loopBack?.index===i}<span title="Loop returns here">↳</span>{/if}</button></th>
        <td class="trace-action"><button onclick={()=>select(i)} title={`Select step ${i}`}>{step.action?.name??'initial'}</button><small title={i===0?'Initial state':`${changes} values changed`}>{i===0?'init':changes?`Δ${changes}`:'unchanged'}</small>{#if i===trace.length-1&&!loopBack}<span class="trace-end" title="Violation occurs here">!</span>{/if}</td>
        {#each columns as field}{@const value=rows[i].get(field)}{@const before=i?rows[i-1].get(field):undefined}<td class:delta={i>0&&value!==before} title={i>0&&value!==before?`${field}: ${before??'absent'} → ${value??'removed'}`:`${field}: ${value??'absent'}`}><button onclick={()=>select(i)}>{value??'—'}</button></td>{/each}
      </tr>
    {/each}</tbody>
  </table>
</div>
{#if loopBack}<div class="trace-loop"><span>↶</span> After the last state, return to state {loopBack.index+1} (step {loopBack.index}){loopBack.stuttering?' and stutter forever.':'; repeat the cycle forever.'}<button onclick={()=>select(loopBack.index)}>Jump to loop</button></div>{/if}
<div class="step-controls"><button disabled={selected===0} onclick={()=>select(selected-1)}>← Previous</button><span>State {selected+1} of {trace.length}</span><button disabled={selected===trace.length-1} onclick={()=>select(selected+1)}>Next →</button>{#if trace[selected].action?.line}<button disabled={stale} onclick={()=>onreveal(trace[selected].action!)}>Show action ↗</button>{/if}{#if selectedGraphId!==undefined}<button onclick={()=>ongraph(selectedGraphId!)}>Graph S{selectedGraphId} ↗</button>{/if}</div>
