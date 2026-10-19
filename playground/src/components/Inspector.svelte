<script lang="ts">
  import type {Binding} from '../lib/types';
  import {stateRows} from '../lib/state-view';
  let {values,previous=[],comparison='previous trace step'}: {values:Binding[];previous?:Binding[];comparison?:string}=$props();
  let changedOnly=$state(false);
  let current=$derived(stateRows(values));
  let old=$derived(new Map(stateRows(previous).map(row=>[row.path,row.value])));
  let all=$derived([...current.map(row=>({...row,before:old.get(row.path),changed:previous.length>0&&old.get(row.path)!==row.value})),...[...old].filter(([path])=>!current.some(row=>row.path===path)).map(([path,before])=>({path,value:'removed',before,changed:true}))]);
  let changed=$derived(all.filter(row=>row.changed).length);
</script>
<div class="inspector">
  <div class="inspector-head"><div><span class="eyebrow">State values</span><span class="comparison">{previous.length?`${changed}/${all.length} changed · vs ${comparison}`:`${all.length} values · no comparison`}</span></div><label><input type="checkbox" bind:checked={changedOnly} disabled={!previous.length}/> Changed only</label></div>
  {#each all.filter(row=>!changedOnly||!previous.length||row.changed) as row}
    <div class:changed={row.changed} class="value-row"><code>{row.path}</code><div>{#if row.changed&&row.before!==undefined}<del>{row.before}</del><span class="value-arrow"> → </span>{/if}<strong>{row.value}</strong></div></div>
  {/each}
  {#if changedOnly&&previous.length&&changed===0}<p class="inspector-empty">No values changed.</p>{/if}
</div>
