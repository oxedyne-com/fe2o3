"use strict";

/* ---- icons (inline, self-contained) ---- */
const P = {
  plus:'<path d="M12 5v14M5 12h14"/>',
  book:'<path d="M5 4.5h12v15H5z"/><path d="M9 4.5v15"/>',
  branch:'<circle cx="6" cy="5" r="2.4"/><circle cx="6" cy="19" r="2.4"/><circle cx="18" cy="8" r="2.4"/><path d="M6 7.4v9.2"/><path d="M6 13c6 0 10-1.6 11-4.7"/>',
  chevron:'<path d="M6 9l6 6 6-6"/>',
  pencil:'<path d="M4 20l4-1L20 7l-3-3L7 16z"/>',
  arrowUp:'<path d="M12 19V5M6 11l6-6 6 6"/>',
  check:'<path d="M5 12l5 5L20 7"/>',
  alert:'<path d="M12 4L2 20h20z"/><path d="M12 10v5M12 17.5h.01"/>',
  slash:'<path d="M17 5L7 19"/>',
  folder:'<path d="M3 6h6l2 2h10v11H3z"/>',
  file:'<path d="M7 3h7l4 4v14H7z"/><path d="M14 3v4h4"/>',
  fileText:'<path d="M7 3h7l4 4v14H7z"/><path d="M14 3v4h4M10 13h5M10 17h4"/>',
  history:'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l4 2"/>',
  users:'<circle cx="9" cy="8" r="3"/><path d="M3 20c0-3.3 2.7-5 6-5s6 1.7 6 5"/><path d="M16 5.6a3 3 0 010 4.8M15.5 15c2.6.3 4.5 2 4.5 5"/>',
  trash:'<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  refresh:'<path d="M4 12a8 8 0 0113.7-5.7L20 8M20 3.5v4.5h-4.5"/><path d="M20 12a8 8 0 01-13.7 5.7L4 16M4 20.5V16h4.5"/>',
  message:'<path d="M4 5h16v10H9l-4 4v-4H4z"/>'
};
const icon = (n, cls='ic') => `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true">${P[n]||''}</svg>`;

/* ---- state ---- */
const state = {
  active: 'ct',
  view: { type:'brief' },
  projects: [
    { id:'ct', name:'Cheap Thinking', icon:'book', team:false, meter:'42% · $0.38' },
    { id:'ox', name:'Oxegen',        icon:'branch', team:true,  meter:'team · $1.72' }
  ],
  chats: [
    { id:'c1', project:'ct', title:'regime framing?' },
    { id:'c2', project:'ct', title:'title ideas' }
  ],
  briefs: {
    ct: { title:'Cheap Thinking', thin:'brief · 42% · $0.38' },
    ox: { title:'Oxegen · conductor', thin:'thin · 9% · $0.14', team:true, conflict:true }
  },
  agents: {
    ct: [
      { id:'a1', name:'voice · ch 7', kind:'light', ctx:34, cost:'0.06', line:'rebuilding scene two…' },
      { id:'a2', name:'figures · ch 3', kind:'exa', ctx:58, cost:'0.11', line:'reading source 7 of 12…' },
      { id:'a3', name:'restructure', kind:'heavy', ctx:71, cost:'0.19', line:'recompiling the pdf…' },
      { id:'a4', name:'trim · ch 5', kind:'done', note:'cut 320 words', cost:'0.04' }
    ],
    ox: [
      { id:'w1', name:'w1 · token', kind:'heavy', branch:'@token-refactor', ctx:71, cost:'0.42' },
      { id:'w3', name:'w3 · signup', kind:'light', branch:'@signup-fix', ctx:38, cost:'0.11' },
      { id:'w2', name:'w2 · identity', kind:'conflict', ctx:24, cost:'0.07' },
      { id:'w4', name:'w4 · identity', kind:'conflict', ctx:22, cost:'0.06' },
      { id:'w5', name:'w5 · rate-limit', kind:'queued', note:'held by governor' }
    ]
  },
  trees: {
    ct: [
      ['shared/',0,'folder',0], ['app.css',1,'file',0], ['brief.js',1,'file',0],
      ['cheap-thinking/',0,'folder',0], ['brief.md',1,'fileText',1], ['panel/',1,'folder',0],
      ['chapters/',1,'folder',0], ['ch03.typ',2,'file',0], ['ch05.typ',2,'file',0], ['ch07.typ',2,'file',0],
      ['compile.sh',1,'file',0]
    ],
    ox: [
      ['oxegen/',0,'folder',0], ['brief.md',1,'fileText',1], ['panel/',1,'folder',0],
      ['fe2o3/',1,'folder',0,'w1'], ['www/',1,'folder',0,'w3'], ['identity.rs',1,'file',0,'warn'],
      ['.red/log',1,'history',0], ['shared/',0,'folder',0]
    ]
  }
};

/* ---- helpers ---- */
const el = id => document.getElementById(id);
const proj = id => state.projects.find(p => p.id === id);
let seq = 100;

/* ---- render: rail ---- */
function renderRail(){
  const cur = state.active;
  const projHTML = state.projects.map(p => `
    <button class="proj ${p.id===cur?'on':''}" onclick="pick('${p.id}')">
      <span class="nm">${icon(p.icon)}${p.name}</span>
      <span class="mt">${p.meter}</span>
    </button>`).join('');

  const chatsHTML = state.chats.map(c => `
    <button class="chat ${state.view.type==='chat'&&state.view.id===c.id?'on':''}" onclick="openChat('${c.id}')">
      <span class="t">${c.title}</span>
      ${c.context?'<span class="dot" title="brief loaded"></span>':''}
    </button>`).join('');

  el('rail').innerHTML = `
    <div class="railhead"><span>Projects</span>
      <button class="addbtn" aria-label="New project" onclick="addProject()">${icon('plus')}</button></div>
    ${projHTML}
    <div class="railhead" style="margin-top:8px"><span>Chats</span>
      <button class="addbtn" aria-label="New chat" onclick="addChat()">${icon('plus')}</button></div>
    ${chatsHTML || '<div class="muted" style="padding:4px 6px;font-size:12px">No chats — fold or dispose keeps this short.</div>'}
  `;
}

/* ---- render: center ---- */
function renderCenter(){
  if (state.view.type === 'chat') return renderChatView();
  return renderBriefView();
}

function renderBriefView(){
  const p = proj(state.active); const b = state.briefs[state.active];
  let head = `
    <div class="chead">
      <div class="ctitle" contenteditable="true" spellcheck="false">${b.title}</div>
      <div style="display:flex;align-items:center;gap:8px">
        <button class="ghost" onclick="chatFromBrief()">${icon('message')} Chat</button>
        <span class="cmeter">${b.thin}</span>
      </div>
    </div>`;

  let config = b.team ? `
    <div class="config">
      <span class="chip">strategy: parallel ${icon('chevron')}</span>
      <span class="chip">governor: 3 heavy ${icon('chevron')}</span>
    </div>` : '';

  let banner = (b.team && b.conflict) ? `
    <div class="banner" id="banner">
      <span class="msg">${icon('alert')} w2 and w4 both edited <code>identity.rs</code></span>
      <button onclick="resolveConflict()">Resolve</button>
    </div>` : '';

  let body = b.team ? boardBody() : proseBody();

  let foot = `
    <div class="cfoot">
      <span class="slash">${icon('slash')}</span>
      <input placeholder="${b.team?'/plan the www migration  —  steer the team':'/polish chapter 3  —  steer the brief'}">
      <button class="send" aria-label="Send">${icon('arrowUp')}</button>
    </div>`;

  el('center').innerHTML = head + config + banner + `<div class="cbody">${body}</div>` + foot;
}

function proseBody(){
  return `
    <p class="voice">A fifth regime transition.</p>
    <p class="sub">AI changes how we think, not just what we make.</p>
    <div class="tags">
      <span class="tag ok">ch 3 done</span>
      <span class="tag warn">ch 5 drafted</span>
      <span class="tag n">ch 7 in a run</span>
    </div>
    <div class="lbl">Decisions</div>
    <div class="dec">· regime framing anchors every chapter open<br>· conductor defined once, early<br>· demonstrate, don't assert</div>`;
}

function boardBody(){
  return `
    <div class="lbl">Task board</div>
    <div class="board">
      <div class="trow"><span>refactor token module · <code>fe2o3</code></span>
        <span class="st"><span class="owner">w1</span><span class="status">running</span></span></div>
      <div class="trow"><span>fix signup headless test · <code>www</code></span>
        <span class="st"><span class="owner">w3</span><span class="status">running</span></span></div>
      <div class="trow" id="t-blocked"><span>update www token consumers</span>
        <span class="status mut" id="t-blocked-s">blocked on w1</span></div>
      <div class="trow"><span>add oxenym rate limit</span>
        <span class="status mut">unclaimed</span></div>
    </div>`;
}

function renderChatView(){
  const c = state.chats.find(x => x.id === state.view.id);
  if (!c){ state.view={type:'brief'}; return renderCenter(); }
  const p = proj(c.project);
  const thread = (c.thread||[]).map(m =>
    m.sys ? `<div class="ctx-note">${icon('book')} ${m.text}</div>`
          : `<div class="msg ${m.role}">${m.text}</div>`).join('');
  el('center').innerHTML = `
    <div class="chead">
      <div class="ctitle">${c.title}</div>
      <div style="display:flex;align-items:center;gap:8px">
        <span class="cmeter">${p?p.name:''} brief · context</span>
        <button class="ghost" onclick="deleteChat('${c.id}')">${icon('trash')} Delete</button>
      </div>
    </div>
    <div class="cbody"><div class="thread">${thread || '<div class="muted" style="font-size:13px">Empty chat — start exploring.</div>'}</div></div>
    <div class="cfoot">
      <span class="slash">${icon('slash')}</span>
      <input placeholder="explore an idea…">
      <button class="send" aria-label="Send">${icon('arrowUp')}</button>
    </div>`;
}

/* ---- render: agents ---- */
function renderAgents(){
  const list = state.agents[state.active] || [];
  const live = list.filter(a => a.kind!=='done' && a.kind!=='queued').length;
  const queued = list.filter(a => a.kind==='queued').length;
  const tiles = list.map(a => {
    if (a.kind === 'done') return `
      <div class="acard done" id="ag-${a.id}">
        <div class="ah"><span class="an">${a.name}</span>
          <span class="status" style="color:var(--ok);display:flex;align-items:center;gap:3px">${icon('check')} done</span></div>
        <div class="arow" style="margin-bottom:6px"><span>${a.note||''}</span><span>$${a.cost}</span></div>
        <div class="abtns"><button onclick="fold('${a.id}')">Fold in</button><button>Diff</button></div>
      </div>`;
    if (a.kind === 'queued') return `
      <div class="acard queued"><div class="ah"><span class="an" style="color:var(--text-2)">${a.name}</span>
        <span class="status mut">queued</span></div>
        <div class="arow"><span>${a.note||''}</span></div></div>`;
    const badge = a.kind==='heavy' ? '<span class="pill heavy">heavy</span>'
      : a.kind==='exa' ? '<span class="pill" style="color:var(--accent-text);background:var(--accent-bg)">Exa</span>'
      : a.kind==='conflict' ? '<span class="status" style="color:var(--warn)">conflict</span>'
      : '<span class="pill" style="color:var(--text-2);background:var(--surface-2)">light</span>';
    return `
      <div class="acard ${a.kind==='conflict'?'conflict':''}" id="ag-${a.id}">
        <div class="ah"><span class="an">${a.name}</span>${badge}</div>
        ${a.branch?`<div class="br">${a.branch}</div>`:''}
        <div class="bar ${a.kind==='heavy'?'h':''}"><i style="width:${a.ctx}%"></i></div>
        <div class="arow"><span>${a.ctx}%</span><span>$${a.cost}</span></div>
      </div>`;
  }).join('');
  el('agents').innerHTML = `
    <div class="railhead"><span>Agents</span>
      <span class="muted" style="font-family:var(--mono);font-size:11px">${live} live${queued?` · ${queued} q`:''}</span></div>
    ${tiles}`;
}

/* ---- render: workspace ---- */
function renderWork(){
  const rows = (state.trees[state.active]||[]).map(r => {
    const [name, depth, ic, hl, badge] = r;
    const right = badge==='warn'
      ? `<span style="color:var(--warn)">${icon('alert','ic')}</span>`
      : badge ? `<span class="badge">${badge}</span>` : '';
    return `<div class="tnode ${hl?'hl':''}">
      <span class="l" style="padding-left:${depth*14}px">${icon(ic)}${name}</span>${right}</div>`;
  }).join('');
  el('work').innerHTML = `
    <div class="railhead"><span>Workspace</span></div>
    <div class="tree">${rows}</div>`;
}

/* ---- render: topbar ---- */
function renderTop(){
  const team = state.active==='ox';
  el('meter').innerHTML = team
    ? `${icon('users')} <span>5 agents</span><span class="sep">·</span><span>63% budget · $2.10</span><span class="pill heavy">heavy 1/3</span>`
    : `${icon('refresh')} <span>session 58% ctx</span><span class="sep">·</span><span>$0.71</span>`;
}

function renderAll(){ renderTop(); renderRail(); renderCenter(); renderAgents(); renderWork(); }

/* ---- actions ---- */
function pick(id){ state.active = id; state.view = {type:'brief'}; renderAll(); }

function addProject(){
  const id = 'p'+(seq++);
  state.projects.push({ id, name:'New project', icon:'folder', team:false, meter:'0% · $0.00' });
  state.briefs[id] = { title:'New project', thin:'brief · 0% · $0.00' };
  state.agents[id] = [];
  state.trees[id] = [[id+'/',0,'folder',0],['brief.md',1,'fileText',1],['panel/',1,'folder',0]];
  state.active = id; state.view = {type:'brief'};
  renderAll();
}

function addChat(){
  const id = 'c'+(seq++);
  state.chats.unshift({ id, project:state.active, title:'New chat', thread:[] });
  state.view = {type:'chat', id};
  renderAll();
}

function chatFromBrief(){
  const p = proj(state.active);
  const id = 'c'+(seq++);
  state.chats.unshift({
    id, project:state.active, title:`${p.name} — chat`, context:true,
    thread:[{ sys:true, text:`Loaded ${p.name} brief as context.` }]
  });
  state.view = {type:'chat', id};
  renderAll();
}

function openChat(id){ state.view = {type:'chat', id}; renderAll(); }

function deleteChat(id){
  state.chats = state.chats.filter(c => c.id !== id);
  state.view = {type:'brief'};
  renderAll();
}

function fold(agentId){
  const card = el('ag-'+agentId);
  if (card){
    card.style.transition='opacity .35s, transform .35s';
    card.style.opacity='0'; card.style.transform='translateX(16px)';
  }
  setTimeout(() => {
    state.agents[state.active] = state.agents[state.active].filter(a => a.id !== agentId);
    renderAgents();
  }, 360);
}

function resolveConflict(){
  const b = el('banner');
  if (b){ b.style.transition='opacity .3s'; b.style.opacity='0'; setTimeout(()=>b.remove(),300); }
  state.briefs.ox.conflict = false;
  state.agents.ox = state.agents.ox.map(a =>
    a.kind==='conflict' ? {...a, kind:'light', name:a.name, cost:a.cost, ctx:a.ctx, merged:true} : a);
  // reflect merge + unblock without full re-render of the banner
  const bl = el('t-blocked-s'); if (bl){ bl.textContent='running'; bl.classList.remove('mut'); }
  renderAgents(); renderWork();
}

document.addEventListener('DOMContentLoaded', renderAll);
