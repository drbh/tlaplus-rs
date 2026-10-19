<script lang="ts">
  import {onMount} from 'svelte';
  import {EditorView,basicSetup} from 'codemirror';
  import {keymap} from '@codemirror/view';
  import {Compartment,Prec,EditorState} from '@codemirror/state';
  import {indentWithTab} from '@codemirror/commands';
  import {setDiagnostics} from '@codemirror/lint';
  import {modelLanguage,modelColors} from '../lib/language';
  import type {Language,Result} from '../lib/types';
  let {source,language,onedit=()=>{},onrun=()=>{},problem,readonly=false}: {source:string;language:Language;onedit?:(s:string)=>void;onrun?:()=>void;problem?:Result;readonly?:boolean}=$props();
  let parent:HTMLDivElement; let editor:EditorView|undefined=$state.raw();
  let wrap=$state(false),fontSize=$state(13),position=$state('Ln 1, Col 1');
  const syntax=new Compartment(),presentation=new Compartment(); let loading=false;
  function appearance(){return [EditorView.theme({
    '&':{height:'100%',background:'#10151f',color:'#d5ddeb'},
    '.cm-scroller':{overflow:'auto',fontFamily:'var(--mono)',fontSize:`${fontSize}px`,lineHeight:'1.75'},
    '.cm-content':{padding:'16px 0',caretColor:'#83e2c2'},
    '.cm-line':{padding:'0 20px 0 12px'},
    '.cm-gutters':{background:'#10151f',color:'#627189',borderRight:'none',paddingRight:'8px'},
    '.cm-activeLine, .cm-activeLineGutter':{background:'#1c283b66'},
    '.cm-cursor':{borderLeftColor:'#83e2c2'},
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection':{background:'#36547488'},
    '.cm-searchMatch':{background:'#80642f88',outline:'1px solid #ba975e'},
    '.cm-panels':{background:'#1a2434',color:'#d5ddeb'},
    '.cm-tooltip':{background:'#1a2434',color:'#d5ddeb',border:'1px solid #3b4b63'},
    '.cm-foldPlaceholder':{background:'#25354a',color:'#b9d1ed',border:'none'}
  },{dark:true}),...(wrap?[EditorView.lineWrapping]:[])];}
  onMount(()=>{
    editor=new EditorView({parent,doc:source,extensions:[basicSetup,modelColors,syntax.of(modelLanguage(language)),presentation.of(appearance()),
      EditorState.readOnly.of(readonly),EditorView.editable.of(!readonly),EditorView.contentAttributes.of({'aria-label':readonly?'Generated TLA+ source':'Model source','spellcheck':'false'}),
      Prec.highest(keymap.of([{key:'Mod-Enter',run:()=>{onrun();return true}},indentWithTab])),
      EditorView.updateListener.of(u=>{
        if(u.docChanged&&!loading)onedit(u.state.doc.toString());
        if(u.selectionSet||u.docChanged){const at=u.state.selection.main.head,line=u.state.doc.lineAt(at);position=`Ln ${line.number}, Col ${at-line.from+1}`;}
      })]});
    return()=>editor?.destroy();
  });
  $effect(()=>{if(editor&&source!==editor.state.doc.toString()){loading=true;editor.dispatch({changes:{from:0,to:editor.state.doc.length,insert:source}});loading=false}});
  $effect(()=>{editor?.dispatch({effects:syntax.reconfigure(modelLanguage(language))})});
  $effect(()=>{const extensions=appearance();editor?.dispatch({effects:presentation.reconfigure(extensions)})});
  $effect(()=>{if(editor){const p=problem;const line=p?.location&&editor.state.doc.line(Math.min(p.location.line,editor.state.doc.lines));editor.dispatch(setDiagnostics(editor.state,line?[{from:line.from,to:line.to,severity:'error',message:p!.message}]:[]))}});
  export function reveal(line:number,column=1){if(!editor)return;const l=editor.state.doc.line(Math.min(line,editor.state.doc.lines));const at=Math.min(l.to,l.from+column-1);editor.dispatch({selection:{anchor:at},effects:EditorView.scrollIntoView(at,{y:'center'})});editor.focus()}
</script>
<div class="editor" bind:this={parent}></div>
{#if !readonly}<div class="editor-status"><span>{position}</span><span class="editor-hint">⌘/Ctrl F to find</span><button class:enabled={wrap} aria-pressed={wrap} onclick={()=>wrap=!wrap} title="Toggle line wrapping">Wrap</button><button aria-label="Decrease editor font size" disabled={fontSize<=11} onclick={()=>fontSize--}>A−</button><button aria-label="Increase editor font size" disabled={fontSize>=20} onclick={()=>fontSize++}>A+</button></div>{/if}
<style>
.editor{flex:1;min-height:0;overflow:hidden}.editor-status{display:flex;align-items:center;gap:12px;min-height:33px;padding:0 16px;border-top:1px solid var(--line);font-size:10px;color:var(--muted);background:#111925}.editor-status>span:first-child{margin-right:auto}.editor-status button{padding:3px 5px;border:0;background:transparent;font-size:10px}.enabled{color:var(--accent)}@media(max-width:500px){.editor-hint{display:none}}
</style>
