<script lang="ts">
  import type {Binding,Value} from '../lib/types';
  let {values,previous=[]}: {values:Binding[];previous?:Binding[]}=$props();
  function rows(bindings:Binding[]) {
    const result:{path:string;value:string}[]=[];
    function visit(path:string,value:Value){
      if(value.kind==='function' && (value.data as unknown[]).length) {
        for(const [key,child] of value.data as [Value,Value][])visit(`${path}[${key.text}]`,child);
      } else result.push({path,value:value.text});
    }
    for(const item of bindings)visit(item.name,item.value);
    return result;
  }
  let current=$derived(rows(values)); let old=$derived(new Map(rows(previous).map(r=>[r.path,r.value])));
</script>
<div class="inspector">
  <div class="eyebrow">State values <span>· changed values highlighted</span></div>
  {#each current as row}
    <div class:changed={previous.length>0&&old.get(row.path)!==row.value} class="value-row">
      <code>{row.path}</code><div>{#if previous.length>0&&old.has(row.path)&&old.get(row.path)!==row.value}<del>{old.get(row.path)}</del> → {/if}<strong>{row.value}</strong></div>
    </div>
  {/each}
  {#each [...old].filter(([path])=>!current.some(row=>row.path===path)) as [path,value]}
    <div class="value-row changed"><code>{path}</code><div><del>{value}</del> → removed</div></div>
  {/each}
</div>
