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
let token='', selected=null, generation=0, lastAnsi=null, control=null;
let actionPending=false, follow=true, collapsed=true, listSignature='', draftKey=null;
const drafts=new Map();
const incoming=new URLSearchParams(location.hash.slice(1)).get('token');
history.replaceState(null,'',location.pathname);
let drawerCloseTimer;
function setCollapsed(value) {
  collapsed=value;
  clearTimeout(drawerCloseTimer);
  const drawer=$('agent-drawer');
  $('toggle-agents').setAttribute('aria-expanded',String(!value));
  if(value) {
    drawer.classList.remove('shown');
    drawerCloseTimer=setTimeout(()=>{if(collapsed&&drawer.open){drawer.close();$('toggle-agents').focus();}},180);
  } else {
    if(!drawer.open) drawer.showModal();
    requestAnimationFrame(()=>{if(!collapsed) drawer.classList.add('shown');});
  }
}
$('toggle-agents').onclick=()=>setCollapsed(!collapsed);
$('close-agents').onclick=()=>setCollapsed(true);
$('agent-drawer').oncancel=event=>{event.preventDefault();setCollapsed(true);};
$('agent-drawer').onclick=event=>{
  const bounds=$('agent-drawer').getBoundingClientRect();
  if(event.target===$('agent-drawer')&&(event.clientX<bounds.left||event.clientX>bounds.right||event.clientY<bounds.top||event.clientY>bounds.bottom)) setCollapsed(true);
};
$('latest').onclick=()=>{follow=true;$('screen').scrollTop=$('screen').scrollHeight;};
$('screen').onscroll=()=>{const el=$('screen');follow=el.scrollHeight-el.scrollTop-el.clientHeight<32;};
function buttons() {
  const enabled=!!token&&!!selected&&!!control?.available&&!actionPending;
  $('prompt').disabled=!enabled; $('send').disabled=!enabled||!$('prompt').value.trim();
  $('interrupt').disabled=!enabled;
  $('control-hint').textContent=!selected?'':control?.available?(control.mode==='acp'?'ACP会話を操作':'中断はCtrl+Cを送信'):'このAgentの操作接続は利用できません';
}
function updateDraft(nextKey) {
  if(draftKey!==null) drafts.set(draftKey,$('prompt').value);
  if(nextKey!==draftKey) {draftKey=nextKey;$('prompt').value=drafts.get(nextKey)||'';}
}
$('prompt').oninput=()=>{if(draftKey!==null) drafts.set(draftKey,$('prompt').value);buttons();};
$('prompt').onkeydown=event=>{if(event.ctrlKey&&event.key==='Enter'){event.preventDefault();if(!$('send').disabled) sendAction('send');}};
async function api(path, signal) {
  const result=await fetch(path,{headers:{Authorization:`Bearer ${token}`},cache:'no-store',signal});
  if(!result.ok) throw new Error(result.status===401?'トークンが正しくありません':'Agentまたはプレビューを取得できません');
  return result.json();
}
async function sendAction(action) {
  if(actionPending||!selected||!control?.available) return;
  const pane=selected, binding=control.binding, key=draftKey, epoch=generation;
  const text=$('prompt').value;
  if(action==='send'&&!text.trim()) return;
  actionPending=true;buttons();$('action-status').textContent=action==='send'?'送信中…':'中断を要求中…';
  const controller=new AbortController(), timeout=setTimeout(()=>controller.abort(),8000);
  try {
    const response=await fetch('/api/action',{method:'POST',headers:{Authorization:`Bearer ${token}`,'Content-Type':'application/json'},body:JSON.stringify({pane,binding,action,text:action==='send'?text:''}),signal:controller.signal});
    if(!response.ok) throw new Error(response.status===401?'認証に失敗しました':'操作を確認できません。対象と画面を確認してください。');
    if(epoch!==generation||selected!==pane) return;
    if(action==='send') {drafts.delete(key);$('prompt').value='';}
    $('action-status').textContent=action==='send'?'命令を送信しました':'中断を要求しました';
    follow=true;
  } catch(error) {
    if(epoch===generation) $('action-status').textContent=error.name==='AbortError'?'応答がありません。送信済みの可能性があるため、画面を確認してから再操作してください。':error.message;
  } finally {clearTimeout(timeout);actionPending=false;buttons();}
}
$('compose').onsubmit=event=>{event.preventDefault();sendAction('send');};
$('interrupt').onclick=()=>sendAction('interrupt');
async function poll(epoch) {
  if(epoch!==generation||!token) return;
  if(document.hidden) {setTimeout(()=>poll(epoch),1000);return;}
  const controller=new AbortController(), timeout=setTimeout(()=>controller.abort(),5000);
  try {
    const snapshot=await api('/api/agents',controller.signal);
    if(epoch!==generation) return;
    if(!snapshot.panes.some(p=>p.pane_id===selected)) {selected=snapshot.panes[0]?.pane_id??null;lastAnsi=null;$('screen').textContent='';follow=true;}
    control=snapshot.controls?.[selected]||null;
    updateDraft(selected&&control?`${selected}:${control.binding}`:null);buttons();
    const signature=JSON.stringify([snapshot.panes.map(p=>[p.pane_id,p.agent_name,p.agent_kind,p.state,p.session_name,p.window_index,p.pane_index]),selected]);
    if(signature!==listSignature) {
      listSignature=signature;
      const items=snapshot.panes.map(p=>{
        const button=document.createElement('button');button.type='button';
        button.textContent=`${p.agent_name||p.agent_kind} · ${p.state}\n${p.session_name}:${p.window_index}.${p.pane_index}`;
        button.setAttribute('aria-pressed',String(p.pane_id===selected));
        button.onclick=()=>{
          if(actionPending) return;
          updateDraft(null);selected=p.pane_id;control=null;buttons();lastAnsi=null;follow=true;$('screen').textContent='';$('action-status').textContent='';
          setCollapsed(true);generation++;poll(generation);
        };return button;
      });
      $('agents').replaceChildren(...items);
    }
    const pane=snapshot.panes.find(p=>p.pane_id===selected);
    $('label').textContent=pane?`${pane.agent_name||pane.agent_kind} · ${pane.session_name}:${pane.window_index}.${pane.pane_index}`:'実行中のAgentはありません';
    if(selected) {
      const view=await api(`/api/view/${selected}`,controller.signal);
      if(epoch!==generation) return;
      if(view.ansi!==lastAnsi) {const screen=$('screen');renderAnsi(view.ansi,screen);if(follow||lastAnsi===null) screen.scrollTop=screen.scrollHeight;lastAnsi=view.ansi;}
    }
    $('status').textContent='接続中 · '+new Date().toLocaleTimeString();
  } catch(error) {
    if(epoch!==generation) return;
    $('status').textContent=error.name==='AbortError'?'接続タイムアウト · 再接続中':error.message+' · 再試行中';
    control=null;buttons();$('screen').textContent='';lastAnsi=null;
  } finally {clearTimeout(timeout);if(epoch===generation) setTimeout(()=>poll(epoch),1000);}
}
function connect(value) {token=value;generation++;$('pair').hidden=true;$('token').value='';poll(generation);}
$('pair').onsubmit=event=>{event.preventDefault();connect($('token').value.trim());};
$('disconnect').onclick=()=>{
  generation++;token='';selected=null;control=null;lastAnsi=null;listSignature='';draftKey=null;drafts.clear();$('prompt').value='';buttons();
  $('agents').replaceChildren();$('screen').textContent='';$('label').textContent='Agentを選択してください';$('status').textContent='未接続';$('action-status').textContent='';$('pair').hidden=false;
};
if(incoming) connect(incoming);
