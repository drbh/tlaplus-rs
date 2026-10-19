// Lightweight lexical highlighting, not a parser or a type checker.
import {StreamLanguage, HighlightStyle, syntaxHighlighting} from '@codemirror/language';
import {tags} from '@lezer/highlight';
import type {Language} from './types';
const tslKeywords = new Set('spec module import enum struct type where domain state const fn view action init next call require pick in invariant transition property fair weak strong assume if else forall exists recursive choose filter map let'.split(' '));
const tlaKeywords = new Set('MODULE EXTENDS VARIABLE VARIABLES CONSTANT CONSTANTS ASSUME ASSUMPTION THEOREM PROPOSITION LEMMA LOCAL INSTANCE WITH IF THEN ELSE LET IN CASE OTHER CHOOSE UNCHANGED ENABLED DOMAIN SUBSET UNION EXCEPT RECURSIVE INIT NEXT INVARIANT INVARIANTS SPECIFICATION PROPERTY PROPERTIES CONSTRAINT SYMMETRY VIEW TRUE FALSE'.split(' '));
const types = new Set('Int Bool String Set Map Seq Nat BOOLEAN STRING Integers Naturals Sequences FiniteSets TLC'.split(' '));
export function modelLanguage(language:Language){
  return StreamLanguage.define({
    startState:()=>({depth:0}),
    token(stream,state){
      if(stream.eatSpace())return null;
      if(state.depth || stream.match('(*',false)){
        while(!stream.eol()){
          if(stream.match('(*'))state.depth++;
          else if(stream.match('*)')){if(--state.depth===0)break;}
          else stream.next();
        }
        return 'comment';
      }
      if(stream.match(language==='tsl'?'//':'\\*')){stream.skipToEnd();return 'comment';}
      if(stream.match(/"(?:[^"\\]|\\.)*(?:"|$)/))return 'string';
      if(stream.match(/\d+(?:\.\d+)?/))return 'number';
      if(stream.match(/(?:TRUE|FALSE|true|false)\b/))return 'bool';
      if(stream.match(/\\[A-Za-z]+|\\[AE]|[∈∀∃∧∨¬⇒⇔]/))return 'operator';
      const word=stream.match(/[A-Za-z_][A-Za-z_0-9]*/);
      if(word && typeof word!=='boolean'){
        const name=word[0];
        if((language==='tsl'?tslKeywords:tlaKeywords).has(name))return 'keyword';
        if(types.has(name))return 'typeName';
        if(stream.match(/^\s*\(/,false))return 'variableName.function';
        if(language==='tsl'&&/^[A-Z]/.test(name))return 'typeName';
        if(language==='tla'&&stream.match(/^\s*(?:==|≜)/,false))return 'variableName.function';
        return 'variableName';
      }
      if(stream.match(/(?:<=>|=>|->|~>|==|!=|<=|>=|&&|\|\||\/\\|\\\/|\.\.=|::|[+*/%=!<>~^&|:'\-])/))return 'operator';
      if(stream.match(/[{}()[\]]/))return 'bracket';
      stream.next();return 'punctuation';
    },
    languageData:{commentTokens:language==='tsl'?{line:'//'}:{line:'\\*',block:{open:'(*',close:'*)'}}}
  });
}
export const modelColors=syntaxHighlighting(HighlightStyle.define([
  {tag:tags.keyword,color:'#c8a4fa'},
  {tag:tags.typeName,color:'#f1ca85'},
  {tag:tags.function(tags.variableName),color:'#80c7ff'},
  {tag:tags.variableName,color:'#d5ddeb'},
  {tag:tags.operator,color:'#91a8cd'},
  {tag:tags.bool,color:'#f1aa83'},
  {tag:tags.number,color:'#f1aa83'},
  {tag:tags.string,color:'#9cddb6'},
  {tag:tags.comment,color:'#8492a9',fontStyle:'italic'},
  {tag:[tags.bracket,tags.punctuation],color:'#a8b6cc'}
]));
