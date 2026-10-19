import type {Binding,Node} from './types';

export type StateRow = {path:string; value:string};
export function stateRows(bindings:Binding[]):StateRow[] {
  const rows:StateRow[]=[];
  function visit(path:string,value:Binding['value']) {
    if(value.kind==='function' && Array.isArray(value.data) && value.data.length) {
      for(const [key,child] of value.data as [Binding['value'],Binding['value']][]) visit(`${path}[${key.text}]`,child);
    } else rows.push({path,value:value.text});
  }
  for(const binding of bindings)visit(binding.name,binding.value);
  return rows;
}
export function stateKey(values:Binding[]):string {
  return JSON.stringify([...values].sort((a,b)=>a.name.localeCompare(b.name)).map(v=>[v.name,v.value.kind,v.value.data]));
}
export function graphStateIndex(nodes:Node[]):Map<string,number> {
  return new Map(nodes.map(node=>[stateKey(node.values),node.id]));
}
