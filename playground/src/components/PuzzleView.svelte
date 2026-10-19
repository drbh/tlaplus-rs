<script lang="ts">
  import PaxosView from './PaxosView.svelte';
  import type {Binding,Value} from '../lib/types';
  import {lookup,number,name,entries,sequence,field} from '../lib/puzzle-values';
  let {kind,values}:{kind?:string;values:Binding[]}=$props();
  let model=$derived.by(()=>{
    const get=(key:string)=>lookup(values,key);
    if(kind==='knights'){
      const people=entries(get('knight'));
      if(people?.length===3&&people.every(([k,v])=>name(k)&&v.kind==='bool'))return{kind,people};
    }
    if(kind==='features'){
      const enabled=get('enabled');
      if(enabled?.kind==='set'&&Array.isArray(enabled.data)&&(enabled.data as Value[]).every(v=>name(v)))return{kind,features:(enabled.data as Value[]).map(v=>name(v)!)};
    }
    if(kind==='houses'){
      const jobs=sequence(get('job')),pets=sequence(get('pet'));
      if(jobs?.length===3&&pets?.length===3&&[...jobs,...pets].every(v=>name(v)))return{kind,jobs,pets};
    }
    if(kind==='digits'){
      const i=number(get('digitI')),b=number(get('digitB')),l=number(get('digitL'));
      if(i!==undefined&&b!==undefined&&l!==undefined)return{kind,i,b,l};
    }
    if(kind==='sudoku'){
      const board=sequence(get('board'));
      if(board?.length===16&&board.every(v=>{const n=number(v);return n!==undefined&&n>=0&&n<=4}))return{kind,board};
    }
    if(kind==='bridge'){
      const people=entries(get('side')),torch=name(get('torch')),elapsed=number(get('elapsed'));
      if(people?.length===4&&people.every(([k,v])=>name(k)&&['Near','Far'].includes(name(v)??''))&&['Near','Far'].includes(torch??'')&&elapsed!==undefined)return{kind,people,torch,elapsed};
    }
    if(kind==='warehouse'){
      const robot=get('robot'),crate=get('crate');
      const rx=number(field(robot,'x')),ry=number(field(robot,'y')),cx=number(field(crate,'x')),cy=number(field(crate,'y'));
      if([rx,ry,cx,cy].every(n=>n!==undefined&&n>=1&&n<=4))return{kind,rx,ry,cx,cy};
    }
    if(kind==='rota'){
      const shifts=sequence(get('shifts'));if(shifts&&shifts.length<=6&&shifts.every(v=>name(v)))return{kind,shifts};
    }
    if(kind==='factory'){
      const starts=sequence(get('starts'));if(starts&&starts.length<=6&&starts.every(v=>number(v)!==undefined))return{kind,starts};
    }
    return null;
  });
</script>
{#if kind==='paxos'}
  <PaxosView {values}/>
{:else if kind}
  <section class="puzzle-view" aria-label="Puzzle state view">
    <div class="eyebrow">Selected state · puzzle view</div>
    {#if !model}<p>This state does not match this puzzle view’s schema. Use the generic values below.</p>
    {:else if model.kind==='knights'}
      <div class="cards">{#each model.people! as [person,truth]}<div><b>{name(person)}</b><span>{truth.data?'Knight':'Knave'}</span></div>{/each}</div>
    {:else if model.kind==='features'}
      <div class="chips">{#each model.features! as feature}<span>✓ {feature}</span>{/each}</div>
    {:else if model.kind==='houses'}
      <div class="cards">{#each model.jobs! as job,i}<div><small>House {i+1}</small><b>{name(job)}</b><span>{name(model.pets![i])}</span></div>{/each}</div>
    {:else if model.kind==='digits'}
      <div class="equation">{model.i} + {model.b}{model.b} = {model.i}{model.l}{model.l}</div><p>I = {model.i}, B = {model.b}, L = {model.l}</p>
    {:else if model.kind==='sudoku'}
      <div class="sudoku" aria-label="Sudoku board">{#each model.board! as cell,i}<span class:box-right={i%4===1} class:box-bottom={i>=4&&i<8}>{number(cell)||'·'}</span>{/each}</div>
    {:else if model.kind==='bridge'}
      <p>Elapsed: <strong>{model.elapsed} minutes</strong> · Torch: {model.torch}</p><div class="banks">{#each ['Near','Far'] as bank}<div><small>{bank} bank {model.torch===bank?'☀':''}</small>{#each model.people!.filter(([,side])=>name(side)===bank) as [person]}<span>{name(person)}</span>{/each}</div>{/each}</div>
    {:else if model.kind==='warehouse'}
      <div class="warehouse" aria-label="Robot and crate positions">{#each Array.from({length:16},(_,i)=>i) as cell}{@const x=cell%4+1}{@const y=Math.floor(cell/4)+1}<span title={`(${x},${y})`}>{model.rx===x&&model.ry===y?'●':model.cx===x&&model.cy===y?'□':'·'}</span>{/each}</div><p>● robot · □ crate. Floor/walls and goal are defined in the source; this view shows positions only.</p>
    {:else if model.kind==='rota'}
      <table><thead><tr><th>Day</th><th>Morning</th><th>Evening</th></tr></thead><tbody>{#each [0,1,2] as day}<tr><th>{day+1}</th><td>{name(model.shifts![day*2])??'—'}</td><td>{name(model.shifts![day*2+1])??'—'}</td></tr>{/each}</tbody></table>
    {:else if model.kind==='factory'}
      <table><thead><tr><th>Operation</th><th>Start time</th></tr></thead><tbody>{#each [0,1,2,3,4,5] as operation}<tr><th>{operation+1}</th><td>{number(model.starts![operation])??'—'}</td></tr>{/each}</tbody></table><p>Machines, durations, and precedence rules are in the source. Start times alone do not show task duration.</p>
    {/if}
  </section>
{/if}
<style>
  .puzzle-view{margin:12px;padding:12px;border:1px solid var(--line-light);border-radius:0;background:var(--panel)}.puzzle-view p{font-size:11px;line-height:1.6;color:var(--muted)}.cards{display:flex;gap:10px;margin-top:14px}.cards>div{flex:1;padding:12px;background:var(--panel-2);border-radius:4px}.cards b,.cards span,.cards small{display:block}.cards b{font-size:14px;margin:5px 0}.cards span,.cards small{font-size:12px;color:var(--muted)}.chips{display:flex;flex-wrap:wrap;gap:10px;margin-top:14px}.chips span{padding:8px;background:var(--accent-bg);border-radius:4px;color:var(--accent);font-size:12px}.equation{font-family:var(--mono);font-size:26px;margin-top:20px}.sudoku,.warehouse{display:grid;grid-template-columns:repeat(4,1fr);width:220px;margin:16px auto 0;border:2px solid var(--muted)}.sudoku span,.warehouse span{display:grid;place-items:center;height:48px;border:1px solid var(--line);font:20px var(--mono)}.sudoku .box-right{border-right:3px solid var(--muted)}.sudoku .box-bottom{border-bottom:3px solid var(--muted)}.warehouse span{color:var(--accent)}.banks{display:flex;gap:18px}.banks>div{flex:1;min-height:90px;border-top:3px solid var(--accent);background:var(--panel-2);padding:10px}.banks small{display:block;color:var(--muted);margin-bottom:12px}.banks span{display:inline-block;font-size:12px;padding:5px;margin:2px;background:var(--control-hover);border-radius:4px}table{width:100%;margin-top:14px;border-collapse:collapse;text-align:left;font-size:12px}th,td{padding:8px;border-bottom:1px solid var(--line-light)}th{font-weight:500;color:var(--muted)}td{color:var(--ink)}
</style>
