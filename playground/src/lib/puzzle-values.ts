// Decode projections from typed result data. A missing/mismatched field disables
// a view; renderers must not substitute example answers for checker output.
import type {Binding,Value} from './types';
export function lookup(values:Binding[],name:string){return values.find(v=>v.name===name)?.value}
export function number(value:Value|undefined):number|undefined{return value?.kind==='int'&&typeof value.data==='number'?value.data:undefined}
export function name(value:Value|undefined):string|undefined{
  return value&&(value.kind==='string'||value.kind==='model')&&typeof value.data==='string'?value.data.split('::').at(-1):undefined;
}
export function entries(value:Value|undefined):[Value,Value][]|undefined{
  return value?.kind==='function'&&Array.isArray(value.data)?value.data as [Value,Value][]:undefined;
}
export function sequence(value:Value|undefined):Value[]|undefined{
  const items=entries(value);if(!items)return undefined;
  const sorted=[...items].sort((a,b)=>(number(a[0])??0)-(number(b[0])??0));
  return sorted.every(([key],i)=>number(key)===i+1)?sorted.map(([,v])=>v):undefined;
}
export function field(value:Value|undefined,key:string){return entries(value)?.find(([k])=>name(k)===key)?.[1]}
