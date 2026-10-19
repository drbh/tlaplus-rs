import type {Node,Edge} from './types';

// Depth is exploration distance, not a claim that the transition graph is a tree.
export function rankLayout(nodes:Node[]) {
  const levels=new Map<number,Node[]>();
  for(const node of nodes){const level=levels.get(node.depth)??[];level.push(node);levels.set(node.depth,level)}
  const positions=new Map<number,{x:number;y:number}>();
  const ranks=[...levels].sort(([a],[b])=>a-b);
  for(const [,level] of ranks)level.sort((a,b)=>a.id-b.id);
  for(const [rank,[,level]] of ranks.entries()){
    level.sort((a,b)=>(positions.get(a.parent??-1)?.x??0)-(positions.get(b.parent??-1)?.x??0)||a.id-b.id);
    level.forEach((node,lane)=>positions.set(node.id,{x:68+lane*76,y:28+rank*46}));
  }
  return {positions,ranks:ranks.map(([depth],i)=>({depth,y:28+i*46})),width:Math.max(320,Math.max(0,...ranks.map(([,level])=>level.length))*76+80),height:Math.max(100,ranks.length*46+10)};
}
export function neighborhood(nodes:Node[],edges:Edge[],selected:number,limit=80):Node[] {
  const ids=new Set([selected]);
  const incoming=edges.filter(e=>e.to===selected).map(e=>e.from);
  const outgoing=edges.filter(e=>e.from===selected).map(e=>e.to);
  for(let i=0;i<Math.max(incoming.length,outgoing.length)&&ids.size<limit;i++){
    for(const id of [incoming[i],outgoing[i]])if(id!==undefined&&ids.size<limit)ids.add(id);
  }
  return nodes.filter(node=>ids.has(node.id));
}
