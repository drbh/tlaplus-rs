<script lang="ts">
  import type {Binding,Value} from '../lib/types';
  import {lookup,entries,sequence,field,name,number} from '../lib/puzzle-values';
  let {values}:{values:Binding[]}=$props();
  function set(value:Value|undefined){return value?.kind==='set'&&Array.isArray(value.data)?value.data as Value[]:undefined}
  let decoded=$derived.by(()=>{
    const promised=entries(lookup(values,'promised')),proposals=sequence(lookup(values,'proposals'));
    const rawVotes=set(lookup(values,'votes')),rawReplies=set(lookup(values,'promises'));
    if(!promised||!proposals||!rawVotes||!rawReplies)return null;
    const votes=rawVotes.map(v=>({acceptor:name(field(v,'acceptor')),ballot:number(field(v,'ballot')),value:name(field(v,'value'))}));
    const replies=rawReplies.map(v=>({acceptor:name(field(v,'acceptor')),ballot:number(field(v,'ballot')),accepted:number(field(v,'acceptedBallot')),value:name(field(v,'value'))}));
    if(promised.some(([a,b])=>!name(a)||number(b)===undefined)||proposals.some(v=>!name(v))||votes.some(v=>!v.acceptor||v.ballot===undefined||!v.value)||replies.some(v=>!v.acceptor||v.ballot===undefined||v.accepted===undefined||!v.value))return null;
    return{promised,proposals,votes,replies};
  });
</script>
<section class="paxos" aria-label="Paxos state view">
  <div class="eyebrow">Selected state · Paxos</div>
  {#if decoded}
    <table><caption>Durable promises and historical votes</caption><thead><tr><th>Acceptor</th><th>Promised</th>{#each decoded.proposals as _,i}<th>Ballot {i+1} votes</th>{/each}</tr></thead><tbody>
      {#each decoded.promised as [acceptor,ballot]}
        <tr><th>{name(acceptor)}</th><td>{number(ballot)}</td>{#each decoded.proposals as _,i}<td>{decoded.votes.filter(v=>v.acceptor===name(acceptor)&&v.ballot===i+1).map(v=>v.value).join(', ')||'—'}</td>{/each}</tr>
      {/each}
    </tbody></table>
    <div class="proposals">{#each decoded.proposals as value,i}<div><small>Ballot {i+1} proposal</small><strong>{name(value)==='None'?'Not issued':name(value)}</strong><span>{decoded.votes.filter(v=>v.ballot===i+1).length} historical acceptances</span></div>{/each}</div>
    <p>The supplied model needs two matching votes at one ballot to choose a value. A later vote does not erase an earlier choice.</p>
    <details><summary>Promise responses ({decoded.replies.length})</summary><table><thead><tr><th>From</th><th>For ballot</th><th>Prior ballot</th><th>Prior value</th></tr></thead><tbody>{#each decoded.replies as reply}<tr><td>{reply.acceptor}</td><td>{reply.ballot}</td><td>{reply.accepted}</td><td>{reply.value}</td></tr>{/each}</tbody></table></details>
  {:else}<p>This state does not match the Paxos view schema. Use the generic inspector.</p>{/if}
</section>
<style>
  .paxos{margin:12px;padding:12px;background:var(--panel);border:1px solid var(--line-light);border-radius:0;overflow:auto}table{border-collapse:collapse;width:100%;text-align:left;font-size:11px}caption{text-align:left;color:var(--muted);font-size:12px;padding:16px 0 10px}th,td{padding:9px 6px;border-bottom:1px solid var(--line-light);white-space:nowrap}th{color:var(--muted);font-weight:500}td{color:var(--ink)}.proposals{display:flex;gap:12px;margin-top:16px}.proposals>div{flex:1;padding:12px;background:var(--panel-2);border-radius:4px}.proposals small,.proposals strong,.proposals span{display:block}.proposals small{color:var(--muted);font-size:10px}.proposals strong{margin:8px 0;font-size:16px}.proposals span,p,summary{font-size:11px;color:var(--muted);line-height:1.7}summary{cursor:pointer}
</style>
