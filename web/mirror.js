'use strict';
// Render only SGR attributes. All terminal output becomes text nodes, never HTML.
const palette = ['#45475a','#f38ba8','#a6e3a1','#f9e2af','#89b4fa','#f5c2e7','#94e2d5','#bac2de','#585b70','#f38ba8','#a6e3a1','#f9e2af','#89b4fa','#f5c2e7','#94e2d5','#cdd6f4'];
function color256(n) {
  if (n < 16) return palette[n];
  if (n >= 232) return `rgb(${[0,0,0].map(() => 8 + (n-232)*10).join(',')})`;
  n -= 16; const levels = [0,95,135,175,215,255];
  return `rgb(${[Math.floor(n/36), Math.floor(n/6)%6, n%6].map(i=>levels[i]).join(',')})`;
}
function renderAnsi(text, element) {
  const fragment = document.createDocumentFragment();
  const pattern = /\x1b\[([0-9;:]*)m|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]/g;
  let style = {}, pos = 0;
  const append = value => { if (!value) return; const span=document.createElement('span'); span.textContent=value.replace(/[\x00-\x08\x0b-\x1f\x7f]/g,''); Object.assign(span.style,style); fragment.append(span); };
  for (const match of text.matchAll(pattern)) {
    append(text.slice(pos,match.index)); pos=match.index+match[0].length;
    if (match[1] === undefined) continue;
    const codes=match[1].split(/[;:]/).map(Number);
    for(let i=0;i<codes.length;i++) {
      const c=codes[i];
      if(c===0) style={};
      else if(c===1) style.fontWeight='bold';
      else if(c===3) style.fontStyle='italic';
      else if(c===4) style.textDecoration='underline';
      else if(c===22) delete style.fontWeight;
      else if(c===23) delete style.fontStyle;
      else if(c===24) delete style.textDecoration;
      else if(c===39) delete style.color;
      else if(c===49) delete style.backgroundColor;
      else if(c>=30&&c<=37) style.color=palette[c-30];
      else if(c>=90&&c<=97) style.color=palette[c-90+8];
      else if(c>=40&&c<=47) style.backgroundColor=palette[c-40];
      else if(c>=100&&c<=107) style.backgroundColor=palette[c-100+8];
      else if(c===38||c===48) {
        const key=c===38?'color':'backgroundColor', mode=codes[++i];
        if(mode===5) { const n=codes[++i]; if(n>=0&&n<=255) style[key]=color256(n); }
        else if(mode===2) { const rgb=codes.slice(i+1,i+4); i+=3; if(rgb.length===3&&rgb.every(n=>n>=0&&n<=255)) style[key]=`rgb(${rgb.join(',')})`; }
      }
    }
  }
  append(text.slice(pos)); element.replaceChildren(fragment);
}
const $=id=>document.getElementById(id);
let token='', selected=null, generation=0, lastAnsi=null;
const incoming=new URLSearchParams(location.hash.slice(1)).get('token');
history.replaceState(null,'',location.pathname);
async function api(path, signal) {
  const result=await fetch(path,{headers:{Authorization:`Bearer ${token}`},cache:'no-store',signal});
  if(!result.ok) throw new Error(result.status===401?'トークンが正しくありません':'Agentまたはプレビューを取得できません');
  return result.json();
}
async function poll(epoch) {
  if(epoch!==generation||!token) return;
  if(document.hidden) { setTimeout(()=>poll(epoch),1000); return; }
  const controller=new AbortController(), timeout=setTimeout(()=>controller.abort(),5000);
  try {
    const snapshot=await api('/api/agents',controller.signal);
    if(epoch!==generation) return;
    if(!snapshot.panes.some(p=>p.pane_id===selected)) { selected=snapshot.panes[0]?.pane_id??null; lastAnsi=null; $('screen').textContent=''; }
    const buttons=snapshot.panes.map(p=>{
      const button=document.createElement('button');button.type='button';
      button.textContent=`${p.agent_name||p.agent_kind} · ${p.state}\n${p.session_name}:${p.window_index}.${p.pane_index}`;
      button.setAttribute('aria-pressed',String(p.pane_id===selected));
      button.onclick=()=>{selected=p.pane_id;lastAnsi=null;$('screen').textContent='';generation++;poll(generation);};return button;
    });
    $('agents').replaceChildren(...buttons);
    const pane=snapshot.panes.find(p=>p.pane_id===selected);
    $('label').textContent=pane?(pane.agent_name||pane.agent_kind):'実行中のAgentはありません';
    if(selected) {
      const view=await api(`/api/view/${selected}`,controller.signal);
      if(epoch!==generation) return;
      if(view.ansi!==lastAnsi) { const screen=$('screen'), bottom=screen.scrollHeight-screen.scrollTop-screen.clientHeight<32; renderAnsi(view.ansi,screen); if(bottom||lastAnsi===null) screen.scrollTop=screen.scrollHeight; lastAnsi=view.ansi; }
    }
    $('status').textContent='接続中 · '+new Date().toLocaleTimeString();
  } catch(error) {
    if(epoch!==generation) return;
    $('status').textContent=error.name==='AbortError'?'接続タイムアウト · 再接続中':error.message+' · 再試行中';
    $('screen').textContent='';lastAnsi=null;
  } finally { clearTimeout(timeout); if(epoch===generation) setTimeout(()=>poll(epoch),1000); }
}
function connect(value) {token=value;generation++;$('pair').hidden=true;$('token').value='';poll(generation);}
$('pair').onsubmit=event=>{event.preventDefault();connect($('token').value.trim());};
$('disconnect').onclick=()=>{generation++;token='';selected=null;lastAnsi=null;$('agents').replaceChildren();$('screen').textContent='';$('label').textContent='Agentを選択してください';$('status').textContent='未接続';$('pair').hidden=false;};
if(incoming) connect(incoming);
