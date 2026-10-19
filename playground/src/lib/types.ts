// Version-one result contract: graph identities are local to one run.
export type Language = 'tsl' | 'tla';
export type Value = {kind:string; data:unknown; text:string};
export type Binding = {name:string; value:Value};
export type Action = {name:string; line:number|null; column:number|null};
export type Step = {values:Binding[]; action:Action|null};
export type Node = {id:number; values:Binding[]; depth:number; expanded:boolean; parent:number|null};
export type Edge = {from:number; to:number; action:Action};
export type Result = {
  status:'passed'|'violated'|'incomplete'|'error'|'assumption'|'trivial'; message:string;
  statistics?:{states:number;initial:number;depth:number};
  trace?:Step[]; loopBack?:{index:number;stuttering:boolean}|null;
  graph?:{complete:boolean;quotient:boolean;nodes:Node[];edges:Edge[]};
  generated?:{source:string;config:string};
  location?:{line:number;column:number}; properties?:{name:string;holds:boolean}[];
};
