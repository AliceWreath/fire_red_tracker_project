//! User dashboard page.

use super::*;

// ---------------------------------------------------------------------------
// Dashboard page
// ---------------------------------------------------------------------------

pub(crate) const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Dashboard – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale}
body{font-family:'Segoe UI',system-ui,sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;display:flex}
.sidebar{width:190px;min-width:190px;background:#0d1b30;border-right:1px solid rgba(255,255,255,0.06);padding:.75rem 0;overflow-y:auto;min-height:100vh;flex-shrink:0}
.sidebar-group{margin-bottom:.1rem}
.sidebar-group-label{font-size:.63rem;font-weight:700;text-transform:uppercase;letter-spacing:.7px;color:#4a6080;padding:.55rem 1rem .15rem}
.sidebar a{display:block;padding:.32rem 1rem;font-size:.8rem;color:#8aa;text-decoration:none;border-left:2px solid transparent;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;transition-property:color,background,border-left-color;transition-duration:150ms;transition-timing-function:ease-out}
.sidebar a:hover{background:#1a2a45;color:#eee;border-left-color:#5090e0}
.sidebar a.active{background:#1a2a40;color:#e94560;border-left-color:#e94560;font-weight:600}
.main{flex:1;padding:2rem 1.5rem;min-width:0}
.container{max-width:860px}
h1{font-size:1.5rem;color:#e94560;margin-bottom:1.5rem;display:flex;align-items:center;justify-content:space-between;text-wrap:balance}
h1 a{font-size:.85rem;color:#5090e0;text-decoration:none;transition-property:color;transition-duration:150ms;transition-timing-function:ease-out}
h1 a:hover{text-decoration:underline;color:#70b0ff}
.section{background:#16213e;box-shadow:0 0 0 1px rgba(255,255,255,0.08);border-radius:12px;padding:1.5rem;margin-bottom:1.5rem;transition-property:box-shadow;transition-duration:150ms;transition-timing-function:ease-out}
.section:hover{box-shadow:0 0 0 1px rgba(255,255,255,0.13)}
.section-title{font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem;padding-bottom:.5rem;border-bottom:1px solid #1e3a6e}
.stat-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(130px,1fr));gap:1rem;margin-bottom:.5rem}
.stat-card{background:#0f3460;border-radius:8px;padding:1rem;text-align:center}
.stat-num{font-size:1.8rem;font-weight:700;color:#e94560;line-height:1;font-variant-numeric:tabular-nums}
.stat-label{font-size:.75rem;color:#888;margin-top:.3rem;text-transform:uppercase;letter-spacing:.5px}
.btn{display:inline-block;padding:.4rem 1rem;border:none;border-radius:6px;font-size:.85rem;cursor:pointer;text-decoration:none;line-height:1.4;transition-property:transform,background;transition-duration:150ms;transition-timing-function:ease-out}
.btn:not(:disabled):active{transform:scale(0.96)}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-secondary:hover{background:#253d6a}
.btn-success{background:#1a5c2e;color:#7dce7d;border:1px solid #2d8a2d}
.btn-success:hover{background:#1e6a34}
.btn-danger{background:#5c1a1a;color:#ce7d7d;border:1px solid #8a2d2d}
.btn-danger:hover{background:#6a1e1e}
.btn-connect{background:#0f3a4a;color:#7dd;border:1px solid #1a6a7a}
.btn-connect:hover{background:#145060}
.btn-sm{padding:.28rem .65rem;font-size:.78rem}
.btn-xs{padding:.18rem .5rem;font-size:.72rem}
.page-select{background:#1e3a6e;color:#aad;border:1px solid #2d5499;border-radius:4px;padding:.18rem .5rem;font-size:.72rem;cursor:pointer}
.page-select:focus{outline:none;border-color:#e94560}
table{width:100%;border-collapse:collapse;font-size:.85rem}
th{text-align:left;color:#888;font-weight:600;font-size:.72rem;text-transform:uppercase;letter-spacing:.4px;padding:.4rem .6rem;border-bottom:1px solid #1e3a6e}
td{padding:.42rem .6rem;border-bottom:1px solid rgba(255,255,255,0.04);vertical-align:middle}
tr:hover td{background:rgba(255,255,255,0.03)}
.run-id{color:#5090e0;font-weight:600;font-variant-numeric:tabular-nums}
.deaths{color:#e06060;font-variant-numeric:tabular-nums}
.catches{color:#60d060;font-variant-numeric:tabular-nums}
.badge-owner{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a5c;color:#5090e0;border:1px solid #2d5499;vertical-align:middle;margin-left:.35rem}
.badge-invited{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a1a;color:#7dce7d;border:1px solid #2d8a2d;vertical-align:middle;margin-left:.35rem}
.party-grid{display:flex;flex-wrap:wrap;gap:.6rem}
.party-mon{background:#0f3460;border-radius:8px;padding:.6rem .9rem;min-width:110px;font-size:.82rem;transition-property:box-shadow;transition-duration:150ms;transition-timing-function:ease-out}
.party-mon:hover{box-shadow:0 0 0 1px rgba(255,255,255,0.12)}
.mon-name{font-weight:600;color:#eee}
.mon-species{color:#888;font-size:.75rem}
.mon-level{color:#5090e0;font-size:.75rem;font-variant-numeric:tabular-nums}
.mon-shiny{color:#f0d060;font-size:.7rem;margin-left:.3rem}
.invite-row{display:flex;align-items:center;gap:.7rem;padding:.6rem 0;border-bottom:1px solid rgba(255,255,255,0.05);flex-wrap:wrap}
.invite-row:last-child{border-bottom:none}
.invite-info{flex:1;font-size:.85rem}
.invite-run{color:#5090e0;font-weight:600;font-variant-numeric:tabular-nums}
.invite-from{color:#888;font-size:.78rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input[type=text]{width:100%;padding:.5rem .7rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.9rem;margin-bottom:.8rem;transition-property:border-color;transition-duration:150ms;transition-timing-function:ease-out}
input:focus{outline:none;border-color:#e94560}
.form-row{display:flex;gap:.6rem;align-items:flex-end}
.form-row>*{flex:1;margin-bottom:0}
.form-row .btn{flex:0 0 auto}
.msg{margin-top:.6rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.85rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.empty{color:#666;font-size:.85rem;text-align:center;padding:1.5rem 0}
.loading{color:#888;font-size:.85rem}
.td-actions{text-align:right;white-space:nowrap;display:flex;justify-content:flex-end;gap:3px;flex-wrap:wrap}
.token-row{display:flex;gap:.5rem;align-items:center;margin-top:.6rem}
.token-display{font-family:monospace;font-size:.8rem;color:#7de;background:#0d1b30;box-shadow:0 0 0 1px rgba(255,255,255,0.08);border-radius:6px;padding:.5rem .75rem;word-break:break-all;flex:1;user-select:all;min-height:2.2rem;display:flex;align-items:center}
.token-copied{font-size:.78rem;color:#7dce7d;margin-top:.4rem;display:none}
</style>
</head>
<body>
<nav class="sidebar">
  <div class="sidebar-group">
    <div class="sidebar-group-label">Main</div>
    <a href="/dashboard" class="active">Dashboard</a>
    <a href="/join">Join / Run Select</a>
    <a href="/history">Run History</a>
    <a href="/integrations">Integrations</a>
    <a href="/guide">Guide / Help</a>
    <a href="/about">About</a>
    <a href="/mobile" onclick="localStorage.removeItem('desktop_mode')" style="color:#666;font-size:.75rem">Mobile View</a>
  </div>
  <div class="sidebar-group">
    <div class="sidebar-group-label" id="run-views-label">Run Views (slot 0)</div>
    <a href="/0/party" data-slot>Party</a>
    <a href="/0/routes" data-slot>Routes</a>
    <a href="/0/encounters" data-slot>Encounters</a>
    <a href="/0/caught" data-slot>Caught</a>
    <a href="/0/dead" data-slot>Dead</a>
    <a href="/0/box" data-slot>Box</a>
    <a href="/0/types" data-slot>Type Coverage</a>
    <a href="/0/items" data-slot>Items</a>
    <a href="/0/moves" data-slot>Moves</a>
  </div>
  <div class="sidebar-group">
    <div class="sidebar-group-label">Stats</div>
    <a href="/shiny">Shinies</a>
    <a href="/memorial">Memorial</a>
    <a href="/soullink">Soul Link</a>
    <a href="/timeline">Timeline</a>
    <a href="/species">Species</a>
    <a href="/trainers">Trainers</a>
    <a href="/0/goals/manage" data-slot>Goals</a>
  </div>
  <div class="sidebar-group">
    <div class="sidebar-group-label">OBS Browser Sources</div>
    <a href="/alerts">Alerts</a>
    <a href="/0/deaths">Deaths</a>
    <a href="/0/encounter_count">Enc. Count</a>
    <a href="/0/hp">HP Bars</a>
    <a href="/0/badges">Badges</a>
    <a href="/0/nextgym">Next Gym</a>
    <a href="/0/encounter_table">Enc. Table</a>
    <a href="/0/money">Money</a>
    <a href="/0/playtime">Playtime</a>
    <a href="/0/goals" data-slot>Goals (OBS)</a>
  </div>
  <div class="sidebar-group">
    <div class="sidebar-group-label">Admin</div>
    <a href="/db">DB Viewer</a>
    <a href="/db/query">DB Query</a>
    <a href="/cmd">Commands</a>
  </div>
</nav>
<div class="main">
<div class="container">
<h1><span id="page-title">Dashboard</span> <a href="/join">← Back to Join</a> <button class="btn btn-secondary" style="float:right;font-size:.8rem;padding:.3rem .8rem" onclick="doLogout()">Log Out</button></h1>

<!-- ── Stats overview ──────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">Overview</div>
  <div class="stat-grid" id="stat-grid">
    <div class="stat-card"><div class="stat-num" id="stat-runs">—</div><div class="stat-label">Total Runs</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-catches">—</div><div class="stat-label">Caught</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-deaths">—</div><div class="stat-label">Deaths</div></div>
    <div class="stat-card"><div class="stat-num" id="stat-encounters">—</div><div class="stat-label">Encounters</div></div>
  </div>
</div>

<!-- ── Open runs ───────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title" style="display:flex;align-items:center;justify-content:space-between">
    <span>Open Runs</span>
    <button class="btn btn-primary btn-sm" onclick="doCreateRun()" id="create-run-btn">+ New Run</button>
  </div>
  <div id="create-run-msg" class="msg"></div>
  <div id="open-runs-status" class="loading">Loading…</div>
  <table id="open-runs-table" style="display:none">
    <thead><tr><th>#</th><th>Player</th><th>Started</th><th>Caught</th><th>Deaths</th><th>Invite</th><th></th></tr></thead>
    <tbody id="open-runs-body"></tbody>
  </table>
</div>

<!-- ── Most recent party ───────────────────────────────────────────── -->
<div class="section" id="party-section" style="display:none">
  <div class="section-title">Current Party <span id="party-run-label" style="color:#666;font-size:.8rem;font-weight:400"></span></div>
  <div class="party-grid" id="party-grid"></div>
</div>

<!-- ── Pending invites ─────────────────────────────────────────────── -->
<div class="section" id="invites-section" style="display:none">
  <div class="section-title">Pending Run Invites</div>
  <div id="invites-list"></div>
</div>

<!-- ── Auth token ──────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">Auth Token</div>
  <p style="font-size:.82rem;color:#888;text-wrap:pretty">Use this token to authenticate API calls or add <code style="background:#0d1b30;color:#7de;padding:.1rem .3rem;border-radius:3px;font-size:.78rem">?token=…</code> to any OBS overlay URL.</p>
  <div class="token-row">
    <div class="token-display" id="token-display">••••••••••••••••••••••••••••••••</div>
    <button class="btn btn-secondary btn-sm" id="token-toggle-btn" onclick="toggleToken()">Show</button>
    <button class="btn btn-secondary btn-sm" onclick="copyToken()">Copy</button>
  </div>
  <div class="token-copied" id="token-copied">Copied to clipboard</div>
</div>

<!-- ── Active sessions ─────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title" style="display:flex;align-items:center;justify-content:space-between">
    <span>Active Sessions</span>
    <button class="btn btn-danger btn-sm" onclick="revokeOtherSessions()">Sign out everywhere else</button>
  </div>
  <p style="font-size:.82rem;color:#888;text-wrap:pretty">Every device or browser signed in to your account. Revoke any session you don't recognize.</p>
  <div id="sessions-list" class="loading" style="margin-top:.6rem">Loading…</div>
</div>

</div><!-- /container -->
</div><!-- /main -->

<!-- Invite modal overlay -->
<div id="invite-modal" style="display:none;position:fixed;inset:0;background:rgba(0,0,0,.7);z-index:100;align-items:center;justify-content:center">
  <div style="background:#16213e;box-shadow:0 0 0 1px rgba(255,255,255,0.1),0 8px 32px rgba(0,0,0,0.5);border-radius:12px;padding:1.5rem;width:340px;max-width:95vw">
    <div style="font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem">Invite User to Run <span id="modal-run-id" style="color:#5090e0"></span></div>
    <label for="invite-username">Username to invite</label>
    <input id="invite-username" type="text" placeholder="their username" autocomplete="off">
    <div id="msg-invite" class="msg"></div>
    <div style="display:flex;gap:.6rem;margin-top:.5rem">
      <button class="btn btn-primary" style="flex:1" onclick="submitInvite()">Send Invite</button>
      <button class="btn btn-secondary" onclick="closeInviteModal()">Cancel</button>
    </div>
  </div>
</div>

<script>
const TOKEN_KEY='frt_session';
const CLIENT_IP='__CLIENT_IP__';
const DIRECT_PORT=DEFAULT_PORT;
const DIRECT_ACTIVE=DIRECT_MODE_ACTIVE;
let SESSION=null;
let MODAL_RUN_ID=null;

function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function fmtDate(iso){if(!iso)return'—';try{return new Date(iso).toLocaleDateString();}catch{return iso;}}
function authHdr(){return {};}
function openRunPage(runId,sel){
  const p=sel.value;sel.value='';if(!p)return;
  const url=p==='stats'?'/run/'+runId+'/stats':'/'+p+'?run='+runId;
  fetch('/api/me/active_run',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({run_id:runId})}).catch(()=>{});
  window.open(url,'_blank');
}

async function quickConnect(runId){
  const r=await fetch('/api/direct/connect',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({host:CLIENT_IP,port:DIRECT_PORT,run_id:runId}),
  }).catch(()=>null);
  if(!r){alert('Network error.');return;}
  const d=await r.json();
  if(r.ok){window.open('/overlay?run='+runId,'_blank');}
  else{alert(d.error||'Connection failed.');}
}

async function doLogout(){
  await fetch('/api/logout',{method:'POST'}).catch(()=>null);
  window.location.href='/';
}

async function doEndRun(runId){
  if(!confirm('End run #'+runId+'? This cannot be undone.'))return;
  const r=await fetch('/api/run/'+runId+'/end',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(!r){alert('Network error.');return;}
  if(r.ok){loadDashboard();}
  else{const d=await r.json().catch(()=>({}));alert(d.error||'Could not end run.');}
}

async function doCreateRun(){
  const btn=document.getElementById('create-run-btn');
  const msg=document.getElementById('create-run-msg');
  btn.disabled=true;btn.textContent='Creating…';
  msg.className='msg';msg.style.display='none';
  const r=await fetch('/api/run',{method:'POST',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify({})}).catch(()=>null);
  btn.disabled=false;btn.textContent='+ New Run';
  if(!r){msg.className='msg err';msg.textContent='Network error.';msg.style.display='';return;}
  const d=await r.json().catch(()=>({}));
  if(r.ok){msg.className='msg ok';msg.textContent='Run #'+d.run_id+' created.';msg.style.display='';loadDashboard();}
  else{msg.className='msg err';msg.textContent=d.error||'Could not create run.';msg.style.display='';}
}

function updateSidebarSlot(slot){
  if(slot==null)return;
  document.querySelectorAll('.sidebar a[data-slot]').forEach(a=>{
    a.href=a.getAttribute('href').replace(/\/\d+\//,'/'+slot+'/');
  });
  const lbl=document.getElementById('run-views-label');
  if(lbl)lbl.textContent='Run Views (slot '+slot+')';
}

async function init(){
  if(!localStorage.getItem('desktop_mode')&&(/Android|iPhone|iPad|iPod/i.test(navigator.userAgent)||window.innerWidth<768)){
    window.location.href='/mobile';return;
  }
  const [meR,tokR]=await Promise.all([
    fetch('/api/me').catch(()=>null),
    fetch('/api/me/token').catch(()=>null),
  ]);
  if(!meR||!meR.ok){window.location.href='/join';return;}
  const me=await meR.json();
  document.getElementById('page-title').textContent='Dashboard — '+esc(me.username);
  if(tokR&&tokR.ok){const t=await tokR.json();SESSION=t.token||null;}
  loadDashboard();
  loadSessions();
}

async function loadSessions(){
  const el=document.getElementById('sessions-list');
  const r=await fetch('/api/me/sessions').catch(()=>null);
  if(!r||!r.ok){el.textContent='Could not load sessions.';return;}
  const d=await r.json().catch(()=>({}));
  const list=d.sessions||[];
  if(!list.length){el.innerHTML='<div class="empty">No active sessions.</div>';return;}
  let html='<table><tr><th>Signed in</th><th>From</th><th>Device</th><th>Expires</th><th></th></tr>';
  for(const s of list){
    const created=new Date(s.created_at*1000).toLocaleString();
    const expires=new Date(s.expires_at*1000).toLocaleDateString();
    const ua=s.user_agent?esc(s.user_agent.length>60?s.user_agent.slice(0,60)+'…':s.user_agent):'—';
    html+='<tr><td>'+esc(created)+(s.current?' <span class="badge-owner">this session</span>':'')+'</td>'
      +'<td style="font-variant-numeric:tabular-nums">'+esc(s.ip||'—')+'</td>'
      +'<td style="color:#888;font-size:.78rem">'+ua+'</td>'
      +'<td>'+esc(expires)+'</td>'
      +'<td class="td-actions"><button class="btn btn-danger btn-xs" onclick="revokeSession(\''+esc(s.token_prefix)+'\','+(s.current?'true':'false')+')">Revoke</button></td></tr>';
  }
  el.className='';
  el.innerHTML=html+'</table>';
}

async function revokeSession(prefix,isCurrent){
  if(isCurrent&&!confirm('This is your current session — revoking it signs you out. Continue?'))return;
  const r=await fetch('/api/me/sessions/'+prefix,{method:'DELETE'}).catch(()=>null);
  if(!r){alert('Network error.');return;}
  const d=await r.json().catch(()=>({}));
  if(d.error){alert(d.error);return;}
  if(isCurrent){window.location.href='/';return;}
  loadSessions();
}

async function revokeOtherSessions(){
  if(!confirm('Sign out every other device and browser?'))return;
  const r=await fetch('/api/me/sessions/revoke_others',{method:'POST'}).catch(()=>null);
  if(!r){alert('Network error.');return;}
  const d=await r.json().catch(()=>({}));
  if(d.error){alert(d.error);return;}
  loadSessions();
}

async function loadDashboard(){
  const r=await fetch('/api/me/dashboard',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok){
    document.getElementById('open-runs-status').textContent='Could not load dashboard.';
    return;
  }
  const d=await r.json();
  if(d.error){document.getElementById('open-runs-status').textContent=d.error;return;}
  if(d.my_slot!=null)updateSidebarSlot(d.my_slot);

  // Stats
  const s=d.stats||{};
  document.getElementById('stat-runs').textContent=s.runs??0;
  document.getElementById('stat-catches').textContent=s.catches??0;
  document.getElementById('stat-deaths').textContent=s.deaths??0;
  document.getElementById('stat-encounters').textContent=s.encounters??0;

  // Open runs
  const runs=d.open_runs||[];
  const st=document.getElementById('open-runs-status');
  const tbl=document.getElementById('open-runs-table');
  const tbody=document.getElementById('open-runs-body');
  if(!runs.length){st.textContent='No open runs.';st.style.display='';tbl.style.display='none';}
  else{
    st.style.display='none';tbl.style.display='';
    tbody.innerHTML='';
    for(const run of runs){
      const tr=document.createElement('tr');
      const ownerBadge=run.is_owner
        ?'<span class="badge-owner">owner</span>'
        :'<span class="badge-invited">invited</span>';
      const inviteBtn=run.is_owner
        ?'<button class="btn btn-secondary btn-xs" onclick="openInviteModal('+run.id+')">Invite</button>'
        :'';
      const endBtn=run.is_owner&&run.ended_at==null
        ?'<button class="btn btn-danger btn-xs" onclick="doEndRun('+run.id+')">End Run</button>'
        :'';
      tr.innerHTML=
        '<td><span class="run-id">#'+run.id+'</span>'+ownerBadge+'</td>'
        +'<td>'+esc(run.player_name||'—')+'</td>'
        +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
        +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
        +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
        +'<td>'+inviteBtn+'</td>'
        +'<td class="td-actions">'
          +'<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Live View</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select>'
          +(DIRECT_ACTIVE?' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open live view">Quick Connect</button>':'')
          +(endBtn?' '+endBtn:'')
        +'</td>';
      tbody.appendChild(tr);
    }
  }

  // Recent party
  const party=d.recent_party||[];
  if(party.length&&runs.length){
    const ps=document.getElementById('party-section');
    const pg=document.getElementById('party-grid');
    const rl=document.getElementById('party-run-label');
    rl.textContent='(Run #'+runs[0].id+')';
    pg.innerHTML='';
    for(const mon of party){
      const div=document.createElement('div');
      div.className='party-mon';
      div.innerHTML=
        '<div class="mon-name">'+esc(mon.nickname)+(mon.is_shiny?'<span class="mon-shiny">★</span>':'')+'</div>'
        +'<div class="mon-species">'+esc(mon.species_name)+'</div>'
        +'<div class="mon-level">Lv. '+mon.level+'</div>';
      pg.appendChild(div);
    }
    ps.style.display='';
  }

  // Pending invites
  const invites=d.pending_invites||[];
  if(invites.length){
    const sec=document.getElementById('invites-section');
    const list=document.getElementById('invites-list');
    list.innerHTML='';
    for(const inv of invites){
      const row=document.createElement('div');
      row.className='invite-row';
      row.id='invite-row-'+inv.invite_id;
      row.innerHTML=
        '<div class="invite-info">'
          +'<span class="invite-run">Run #'+inv.run_id+'</span>'
          +' <span style="color:#ccc">'+esc(inv.player_name)+'</span>'
          +'<div class="invite-from">Invited by '+esc(inv.invited_by)+' · '+fmtDate(inv.created_at)+'</div>'
        +'</div>'
        +'<button class="btn btn-success btn-sm" onclick="respondInvite('+inv.run_id+',true,'+inv.invite_id+')">Accept</button>'
        +'<button class="btn btn-danger btn-sm" onclick="respondInvite('+inv.run_id+',false,'+inv.invite_id+')">Decline</button>';
      list.appendChild(row);
    }
    sec.style.display='';
  }
}

function openInviteModal(runId){
  MODAL_RUN_ID=runId;
  document.getElementById('modal-run-id').textContent='#'+runId;
  document.getElementById('invite-username').value='';
  document.getElementById('msg-invite').className='msg';
  document.getElementById('invite-modal').style.display='flex';
  setTimeout(()=>document.getElementById('invite-username').focus(),50);
}
function closeInviteModal(){
  document.getElementById('invite-modal').style.display='none';
  MODAL_RUN_ID=null;
}
async function submitInvite(){
  const uname=document.getElementById('invite-username').value.trim();
  const msg=document.getElementById('msg-invite');
  if(!uname){msg.className='msg err';msg.textContent='Enter a username.';return;}
  const r=await fetch('/api/run/'+MODAL_RUN_ID+'/invite',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({username:uname}),
  }).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    msg.className='msg ok';msg.textContent='Invite sent to '+esc(uname)+'.';
    setTimeout(closeInviteModal,1400);
  }else{
    msg.className='msg err';msg.textContent=d.error||'Failed.';
  }
}
async function respondInvite(runId,accept,inviteId){
  const endpoint=accept?'accept':'decline';
  const r=await fetch('/api/run/'+runId+'/invite/'+endpoint,{
    method:'POST',
    headers:authHdr(),
  }).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('invite-row-'+inviteId);
    if(row)row.remove();
    const sec=document.getElementById('invites-section');
    const list=document.getElementById('invites-list');
    if(!list.children.length)sec.style.display='none';
    if(accept)loadDashboard();
  }
}

let tokenVisible=false;
function toggleToken(){
  tokenVisible=!tokenVisible;
  document.getElementById('token-display').textContent=tokenVisible?(SESSION||'—'):'••••••••••••••••••••••••••••••••';
  document.getElementById('token-toggle-btn').textContent=tokenVisible?'Hide':'Show';
}
function copyToken(){
  if(!SESSION)return;
  const msg=document.getElementById('token-copied');
  const show=()=>{msg.style.display='';setTimeout(()=>{msg.style.display='none';},2000);};
  if(navigator.clipboard){navigator.clipboard.writeText(SESSION).then(show).catch(()=>{});}
  else{const el=document.createElement('textarea');el.value=SESSION;document.body.appendChild(el);el.select();document.execCommand('copy');document.body.removeChild(el);show();}
}

init();
</script>
</body>
</html>"#;

pub(crate) async fn serve_dashboard(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    let client_ip = addr.ip().to_string();
    let default_port = state.connector.as_ref().map(|c| c.default_port).unwrap_or(55355);
    let direct_active = if state.connector.is_some() { "true" } else { "false" };
    let html = DASHBOARD_HTML
        .replace("DIRECT_MODE_ACTIVE", direct_active)
        .replace("DEFAULT_PORT", &default_port.to_string())
        .replace("__CLIENT_IP__", &client_ip);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

pub(crate) const INTEGRATIONS_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Integrations — Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale}
body{font-family:'Segoe UI',system-ui,sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;padding:2rem 1rem}
.container{max-width:780px;margin:0 auto}
h1{font-size:1.4rem;color:#e94560;margin-bottom:.3rem;text-wrap:balance}
.subtitle{font-size:.8rem;color:#556;margin-bottom:2rem;text-wrap:pretty}
.card{background:#16213e;box-shadow:0 0 0 1px rgba(255,255,255,0.08);border-radius:12px;padding:1.5rem;margin-bottom:1.5rem;transition-property:box-shadow;transition-duration:150ms;transition-timing-function:ease-out}
.card:hover{box-shadow:0 0 0 1px rgba(255,255,255,0.13)}
.card h2{font-size:1rem;margin-bottom:.1rem;display:flex;align-items:center;gap:.5rem}
.card .desc{font-size:.78rem;color:#777;margin-bottom:1rem;text-wrap:pretty}
label{display:block;font-size:.82rem;color:#ccc;margin-bottom:.25rem}
input,textarea{width:100%;padding:.45rem .65rem;background:#0f3460;border:1px solid #444;border-radius:5px;color:#eee;font-size:.85rem;margin-bottom:.75rem;transition-property:border-color;transition-duration:150ms;transition-timing-function:ease-out}
input:focus,textarea:focus{outline:none;border-color:#e94560}
textarea{resize:vertical;min-height:60px}
.btn{display:inline-block;padding:.4rem .9rem;border:none;border-radius:6px;font-size:.82rem;cursor:pointer;text-decoration:none;transition-property:transform,background;transition-duration:150ms;transition-timing-function:ease-out}
.btn:active{transform:scale(0.96)}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-del{background:#3a1a1a;color:#ce7d7d;border:1px solid #6a2d2d}
.btn-del:hover{background:#5a2020}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-secondary:hover{background:#253d6a}
.actions{display:flex;gap:.5rem;flex-wrap:wrap;margin-top:.5rem}
.status{margin-top:.5rem;font-size:.78rem;padding:.3rem .6rem;border-radius:4px;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.active-badge{font-size:.7rem;background:#1a4a1a;color:#7dce7d;border:1px solid #2d8a2d;border-radius:4px;padding:.1rem .4rem}
nav{display:flex;gap:1rem;font-size:.85rem;margin-bottom:1.5rem}
nav a{color:#aad;text-decoration:none;transition-property:color;transition-duration:150ms;transition-timing-function:ease-out}nav a:hover{color:#fff}
</style>
</head>
<body>
<div class="container">
<nav><a href="/dashboard">← Dashboard</a></nav>
<h1>Integration Settings</h1>
<p class="subtitle">Per-user Twitch, YouTube, and Discord bots. Each integration runs independently for your runs.</p>

<div class="card" id="card-twitch">
<h2>Twitch IRC Bot <span id="badge-twitch" class="active-badge" style="display:none">active</span></h2>
<p class="desc">Chat commands (!party, !deaths, !shinies, !status) and Channel Points EventSub.</p>
<label>Channel (username)</label><input id="twitch-channel" placeholder="yourchannel">
<label>OAuth Token (oauth:xxxxxxxx)</label><input id="twitch-token" placeholder="oauth:..." type="password">
<label>Client ID (for Channel Points)</label><input id="twitch-client-id" placeholder="optional">
<label>Broadcaster ID (for Channel Points)</label><input id="twitch-broadcaster-id" placeholder="optional">
<div class="actions">
  <button class="btn btn-primary" onclick="saveIntegration('twitch')">Save &amp; Start</button>
  <button class="btn btn-del" onclick="deleteIntegration('twitch')">Remove</button>
</div>
<div class="status" id="status-twitch"></div>
</div>

<div class="card" id="card-youtube">
<h2>YouTube Live Chat Bot <span id="badge-youtube" class="active-badge" style="display:none">active</span></h2>
<p class="desc">Chat commands (!party, !deaths, etc.) for your YouTube Live stream.</p>
<label>API Key</label><input id="youtube-api-key" placeholder="AIza...">
<label>Broadcast ID</label><input id="youtube-broadcast-id" placeholder="video ID">
<label>Poll interval (seconds, min 5)</label><input id="youtube-poll-secs" type="number" min="5" value="15">
<div class="actions">
  <button class="btn btn-primary" onclick="saveIntegration('youtube')">Save &amp; Start</button>
  <button class="btn btn-del" onclick="deleteIntegration('youtube')">Remove</button>
</div>
<div class="status" id="status-youtube"></div>
</div>

<div class="card" id="card-discord_embed">
<h2>Discord Live Embed <span id="badge-discord_embed" class="active-badge" style="display:none">active</span></h2>
<p class="desc">Edits a pinned message in your Discord server with live party/badge info.</p>
<label>Bot Token</label><input id="discord_embed-bot-token" placeholder="Bot token" type="password">
<label>Channel ID</label><input id="discord_embed-channel-id" placeholder="channel snowflake">
<label>Message ID</label><input id="discord_embed-message-id" placeholder="pinned message snowflake">
<label>Update interval (seconds, min 10)</label><input id="discord_embed-interval" type="number" min="10" value="30">
<div class="actions">
  <button class="btn btn-primary" onclick="saveIntegration('discord_embed')">Save &amp; Start</button>
  <button class="btn btn-del" onclick="deleteIntegration('discord_embed')">Remove</button>
</div>
<div class="status" id="status-discord_embed"></div>
</div>

<div class="card" id="card-discord_thread">
<h2>Discord Run Threads <span id="badge-discord_thread" class="active-badge" style="display:none">active</span></h2>
<p class="desc">Creates a new thread in a channel for each run and posts milestone updates.</p>
<label>Bot Token</label><input id="discord_thread-bot-token" placeholder="Bot token" type="password">
<label>Channel ID</label><input id="discord_thread-channel-id" placeholder="channel snowflake">
<div class="actions">
  <button class="btn btn-primary" onclick="saveIntegration('discord_thread')">Save &amp; Start</button>
  <button class="btn btn-del" onclick="deleteIntegration('discord_thread')">Remove</button>
</div>
<div class="status" id="status-discord_thread"></div>
</div>
</div>

<script>
function showStatus(kind,msg,ok){
  const el=document.getElementById('status-'+kind);
  el.textContent=msg;el.className='status '+(ok?'ok':'err');
  setTimeout(()=>el.className='status',4000);
}
function getConfig(kind){
  if(kind==='twitch') return {
    channel:document.getElementById('twitch-channel').value.trim(),
    token:document.getElementById('twitch-token').value.trim(),
    slot:0,
    client_id:document.getElementById('twitch-client-id').value.trim()||null,
    broadcaster_id:document.getElementById('twitch-broadcaster-id').value.trim()||null,
    reward_commands:{},
  };
  if(kind==='youtube') return {
    api_key:document.getElementById('youtube-api-key').value.trim(),
    broadcast_id:document.getElementById('youtube-broadcast-id').value.trim(),
    poll_secs:parseInt(document.getElementById('youtube-poll-secs').value)||15,
    slot:0,
  };
  if(kind==='discord_embed') return {
    bot_token:document.getElementById('discord_embed-bot-token').value.trim(),
    channel_id:parseInt(document.getElementById('discord_embed-channel-id').value)||0,
    message_id:parseInt(document.getElementById('discord_embed-message-id').value)||0,
    update_interval_secs:parseInt(document.getElementById('discord_embed-interval').value)||30,
  };
  if(kind==='discord_thread') return {
    bot_token:document.getElementById('discord_thread-bot-token').value.trim(),
    channel_id:parseInt(document.getElementById('discord_thread-channel-id').value)||0,
  };
  return {};
}
function fillForm(kind,cfg){
  if(kind==='twitch'){
    document.getElementById('twitch-channel').value=cfg.channel||'';
    document.getElementById('twitch-token').value=cfg.token||'';
    document.getElementById('twitch-client-id').value=cfg.client_id||'';
    document.getElementById('twitch-broadcaster-id').value=cfg.broadcaster_id||'';
  }else if(kind==='youtube'){
    document.getElementById('youtube-api-key').value=cfg.api_key||'';
    document.getElementById('youtube-broadcast-id').value=cfg.broadcast_id||'';
    document.getElementById('youtube-poll-secs').value=cfg.poll_secs||15;
  }else if(kind==='discord_embed'){
    document.getElementById('discord_embed-bot-token').value=cfg.bot_token||'';
    document.getElementById('discord_embed-channel-id').value=cfg.channel_id||'';
    document.getElementById('discord_embed-message-id').value=cfg.message_id||'';
    document.getElementById('discord_embed-interval').value=cfg.update_interval_secs||30;
  }else if(kind==='discord_thread'){
    document.getElementById('discord_thread-bot-token').value=cfg.bot_token||'';
    document.getElementById('discord_thread-channel-id').value=cfg.channel_id||'';
  }
}
async function saveIntegration(kind){
  const body=getConfig(kind);
  const r=await fetch('/api/me/integrations/'+kind,{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
  const j=await r.json();
  if(j.ok){showStatus(kind,'Saved and started.',true);document.getElementById('badge-'+kind).style.display='';}
  else showStatus(kind,j.error||'Error',false);
}
async function deleteIntegration(kind){
  if(!confirm('Remove this integration?'))return;
  const r=await fetch('/api/me/integrations/'+kind,{method:'DELETE'});
  const j=await r.json();
  if(j.ok){showStatus(kind,'Removed.',true);document.getElementById('badge-'+kind).style.display='none';}
  else showStatus(kind,j.error||'Error',false);
}
async function loadIntegrations(){
  const r=await fetch('/api/me/integrations');
  if(!r.ok)return;
  const d=await r.json();
  for(const[kind,cfg]of Object.entries(d)){
    const badge=document.getElementById('badge-'+kind);
    if(badge)badge.style.display='';
    fillForm(kind,cfg);
  }
}
loadIntegrations();
</script>
</body>
</html>
"#;

pub(crate) async fn serve_integrations_page(
    State(state): State<WebState>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        apply_page(INTEGRATIONS_HTML, state.testing),
    )
}

pub(crate) const GUIDE_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Guide – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale}
body{font-family:'Segoe UI',system-ui,sans-serif;background:#1a1a2e;color:#eee;display:flex;min-height:100vh}
.sidebar{width:200px;min-width:200px;background:#0d1b30;border-right:1px solid rgba(255,255,255,0.06);padding:.75rem 0;overflow-y:auto;min-height:100vh;flex-shrink:0;position:sticky;top:0;height:100vh}
.sidebar-group-label{font-size:.63rem;font-weight:700;text-transform:uppercase;letter-spacing:.7px;color:#4a6080;padding:.55rem 1rem .15rem}
.sidebar a{display:block;padding:.32rem 1rem;font-size:.8rem;color:#8aa;text-decoration:none;border-left:2px solid transparent;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;transition-property:color,background,border-left-color;transition-duration:150ms;transition-timing-function:ease-out}
.sidebar a:hover{background:#1a2a45;color:#eee;border-left-color:#5090e0}
.sidebar a.back{color:#5090e0;border-bottom:1px solid rgba(255,255,255,0.06);margin-bottom:.4rem;padding-bottom:.5rem}
.main{flex:1;padding:2rem 2.5rem;max-width:860px;min-width:0}
h1{font-size:1.5rem;color:#e94560;margin-bottom:.4rem;text-wrap:balance}
h2{font-size:1.15rem;color:#e94560;margin:2.2rem 0 .8rem;padding-top:.5rem;border-top:1px solid #1e3a6e;text-wrap:balance}
h2:first-of-type{border-top:none;margin-top:1rem}
h3{font-size:.95rem;color:#ccc;margin:1.2rem 0 .4rem;font-weight:700;text-wrap:balance}
p{font-size:.88rem;color:#bbb;line-height:1.6;margin-bottom:.7rem}
ul,ol{font-size:.88rem;color:#bbb;line-height:1.6;margin-bottom:.7rem;padding-left:1.4rem}
li{margin-bottom:.2rem}
a{color:#5090e0;text-decoration:none}
a:hover{text-decoration:underline}
code{background:#0f3460;color:#7de;padding:.1rem .35rem;border-radius:3px;font-size:.82rem;font-family:monospace}
pre{background:#0d1b30;border:1px solid #1e3a6e;border-radius:6px;padding:1rem 1.2rem;margin:.6rem 0 1rem;overflow-x:auto;font-family:monospace;font-size:.82rem;color:#cce;line-height:1.55}
pre code{background:none;padding:0;font-size:inherit}
table{width:100%;border-collapse:collapse;font-size:.83rem;margin-bottom:1rem}
th{text-align:left;color:#888;font-weight:600;font-size:.72rem;text-transform:uppercase;letter-spacing:.4px;padding:.4rem .6rem;border-bottom:1px solid #1e3a6e}
td{padding:.38rem .6rem;border-bottom:1px solid rgba(255,255,255,0.05);vertical-align:top}
tr:hover td{background:rgba(255,255,255,0.02)}
td:first-child{font-family:monospace;color:#7de;white-space:nowrap}
.badge{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;border:1px solid;vertical-align:middle;margin-left:.25rem}
.badge-get{background:#1a3a5c;color:#5090e0;border-color:#2d5499}
.badge-post{background:#1a4a1a;color:#7dce7d;border-color:#2d8a2d}
.badge-put{background:#3a3a1a;color:#d0c060;border-color:#8a8a2d}
.badge-del{background:#4a1a1a;color:#ce7d7d;border-color:#8a2d2d}
.badge-patch{background:#3a1a3a;color:#c07dce;border-color:#8a2d8a}
.note{background:#1a2a45;border-left:3px solid #5090e0;padding:.6rem .9rem;border-radius:0 5px 5px 0;margin:.5rem 0 1rem;font-size:.85rem;color:#aac}
.warn{background:#2a1a1a;border-left:3px solid #e94560;padding:.6rem .9rem;border-radius:0 5px 5px 0;margin:.5rem 0 1rem;font-size:.85rem;color:#caa}
.subtitle{color:#888;font-size:.85rem;margin-bottom:1.5rem}
</style>
</head>
<body>
<nav class="sidebar">
  <a class="back" href="/dashboard">← Dashboard</a>
  <div class="sidebar-group-label">Sections</div>
  <a href="#overview">Overview</a>
  <a href="#auth">Authentication</a>
  <a href="#api">REST API</a>
  <a href="#injections">Injections</a>
  <a href="#websocket">WebSocket Overlay</a>
  <a href="#obs">OBS Integration</a>
  <a href="#twitch">Twitch Bot</a>
  <a href="#youtube">YouTube Chat Bot</a>
  <a href="#discord">Discord</a>
  <a href="#webhooks">Webhooks</a>
  <a href="#per-user">Per-User Integrations</a>
  <a href="#config">Config Reference</a>
</nav>
<div class="main">
<h1>Fire Red Tracker — Guide</h1>
<p class="subtitle">Version __VERSION__</p>

<!-- ── Overview ──────────────────────────────────────────────────────── -->
<h2 id="overview">Overview</h2>
<p>The aggregator runs a single HTTP + WebSocket server. Set <code>ws_port</code> in the config (e.g. <code>ws_port = 9090</code>) to enable it. All pages, the REST API, and the live WebSocket overlay are served from that one port.</p>
<ul>
  <li>Visit <code>http://&lt;host&gt;:9090/</code> to log in.</li>
  <li>Open <code>http://&lt;host&gt;:9090/dashboard</code> after logging in to see your runs and stats.</li>
  <li>Open <code>http://&lt;host&gt;:9090/join</code> to connect a RetroArch instance (direct mode).</li>
  <li>Add browser sources in OBS pointing at overlay URLs like <code>http://&lt;host&gt;:9090/0/party</code>.</li>
</ul>
<div class="note">All page and API requests that return user data require a valid session token. See <a href="#auth">Authentication</a>.</div>

<!-- ── Authentication ─────────────────────────────────────────────────── -->
<h2 id="auth">Authentication</h2>
<p>Create an account at <code>/register</code> and log in at <code>/</code>. The server returns a session token on login.</p>
<h3>Browser</h3>
<p>After login, the server sets an HttpOnly session cookie (<code>frt_token</code>) which is sent automatically on every page and API request. No extra client-side setup needed.</p>
<p>To use the raw token value (e.g. for OBS overlay <code>?token=</code> URLs), fetch it after login:</p>
<pre><code>curl -b cookies.txt http://localhost:9090/api/me/token
# → {"token":"eyJ…"}</code></pre>
<h3>API / curl</h3>
<pre><code># Log in and capture the token
TOKEN=$(curl -s -X POST http://localhost:9090/api/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"secret"}' | jq -r .token)

# Use it on subsequent requests
curl -H "Authorization: Bearer $TOKEN" http://localhost:9090/api/me</code></pre>
<h3>OBS Browser Sources</h3>
<p>Overlay pages also accept a <code>?token=&lt;session_token&gt;</code> query parameter so OBS can load them without a cookie:</p>
<pre><code>http://localhost:9090/0/party?token=YOUR_TOKEN</code></pre>

<!-- ── REST API ────────────────────────────────────────────────────────── -->
<h2 id="api">REST API</h2>
<p>All endpoints live under <code>http://&lt;host&gt;:9090/api/</code>. Authenticated requests must include <code>Authorization: Bearer &lt;token&gt;</code>.</p>

<h3>Account</h3>
<table>
  <thead><tr><th>Method &amp; Path</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>POST /api/login</td><td>Body: <code>{"username","password"}</code>. Returns <code>{"token","user"}</code>. Rate limited: 5 failed attempts per IP per 5 minutes &rarr; <code>429</code> with <code>Retry-After</code>.</td></tr>
    <tr><td>POST /api/logout</td><td>Invalidates the current session token.</td></tr>
    <tr><td>GET /api/me/sessions</td><td>Lists your active sessions (creation time, IP, device, 12-char <code>token_prefix</code> handle).</td></tr>
    <tr><td>DELETE /api/me/sessions/:prefix</td><td>Revokes one session by its <code>token_prefix</code>.</td></tr>
    <tr><td>POST /api/me/sessions/revoke_others</td><td>Revokes every session except the one making the request.</td></tr>
    <tr><td>POST /api/register</td><td>Body: <code>{"username","password"}</code>. Creates a new account.</td></tr>
    <tr><td>GET /api/me</td><td>Returns the current user's profile.</td></tr>
    <tr><td>GET /api/me/token</td><td>Returns the raw session token value as JSON (useful for scripts and OBS URL parameters).</td></tr>
    <tr><td>GET /api/me/dashboard</td><td>Stats, open runs, current party, pending invites.</td></tr>
    <tr><td>GET /api/me/integrations</td><td>List all saved integration configs for the current user.</td></tr>
    <tr><td>PUT /api/me/integrations/:kind</td><td>Save or update an integration config (<code>twitch</code>, <code>youtube</code>, <code>discord_embed</code>, <code>discord_thread</code>, <code>obs</code>). Restarts the thread.</td></tr>
    <tr><td>DELETE /api/me/integrations/:kind</td><td>Remove an integration config and stop its thread.</td></tr>
  </tbody>
</table>

<h3>Runs</h3>
<table>
  <thead><tr><th>Method &amp; Path</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>GET /api/runs</td><td>All runs accessible to the current user.</td></tr>
    <tr><td>POST /api/run</td><td>Create a new run.</td></tr>
    <tr><td>GET /api/run/:id</td><td>Run details including caught, dead, party.</td></tr>
    <tr><td>GET /api/run/:id/summary</td><td>Markdown export of the run.</td></tr>
    <tr><td>GET /api/run/:id/trainers</td><td>Trainer battle log for the run.</td></tr>
    <tr><td>POST /api/run/import</td><td>Import a previously exported run JSON.</td></tr>
  </tbody>
</table>

<h3>Slots (live state)</h3>
<table>
  <thead><tr><th>Method &amp; Path</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>GET /api/state</td><td>Full live state for all slots (party, run metadata, flags).</td></tr>
    <tr><td>GET /api/slot/:index/odds</td><td>Encounter odds for the current area.</td></tr>
    <tr><td>GET /api/slot/:index/bag</td><td>Current bag contents split into pockets: <code>items</code>, <code>key_items</code>, <code>balls</code>, <code>tms</code>. Each entry has <code>item_id</code> and <code>quantity</code>.</td></tr>
    <tr><td>POST /api/slot/:index/command/:cmd</td><td>Fire a slot command: <code>heal_all</code>, <code>new_run</code>, <code>end_run</code>, <code>reset_area</code>.</td></tr>
    <tr><td>POST /api/slot/:index/undo</td><td>Undo the last injection command on this slot.</td></tr>
  </tbody>
</table>

<h3>Injection Commands</h3>
<p>These mutate game state directly via <code>WRITE_CORE_MEMORY</code>. Require <code>allow_injections = true</code> (default) in config. All return <code>403</code> when injections are disabled.</p>
<table>
  <thead><tr><th>Path</th><th>Body</th><th>Effect</th></tr></thead>
  <tbody>
    <tr><td>POST /api/slot/:i/give_item</td><td><code>{"item_id":&lt;u16&gt;,"quantity":&lt;u16 1–99&gt;}</code></td><td>Add items to the bag items pocket.</td></tr>
    <tr><td>POST /api/slot/:i/take_item</td><td><code>{"item_id":&lt;u16&gt;,"quantity":&lt;u16 1–99&gt;}</code></td><td>Remove items from the bag. If the quantity exceeds what's held, the slot is fully removed.</td></tr>
    <tr><td>POST /api/slot/:i/make_shiny</td><td><code>{"party_position":&lt;u8 0–5&gt;}</code></td><td>Patch OT Secret ID so the Gen III shiny formula is satisfied. Preserves nature, ability, gender, and IVs.</td></tr>
    <tr><td>POST /api/slot/:i/change_species</td><td><code>{"party_position":&lt;u8 0–5&gt;,"new_species":&lt;u16 1–386&gt;}</code></td><td>Rewrite species in the Growth substructure; recalculates checksum.</td></tr>
    <tr><td>POST /api/slot/:i/change_ability</td><td><code>{"party_position":&lt;u8 0–5&gt;,"ability_slot":&lt;u8 0 or 1&gt;}</code></td><td>Toggle primary vs. secondary ability.</td></tr>
    <tr><td>POST /api/slot/:i/change_gender</td><td><code>{"party_position":&lt;u8 0–5&gt;,"target_gender":&lt;u8 0 or 1&gt;}</code></td><td>Adjust personality low byte to satisfy target gender (0=male, 1=female).</td></tr>
    <tr><td>POST /api/slot/:i/change_nature</td><td><code>{"party_position":&lt;u8 0–5&gt;,"nature":&lt;u8 0–24&gt;}</code></td><td>Set nature by adjusting personality so <code>personality % 25 == nature</code>.</td></tr>
    <tr><td>POST /api/slot/:i/change_nickname</td><td><code>{"party_position":&lt;u8 0–5&gt;,"nickname":&lt;string&gt;}</code></td><td>Write a new nickname (max 10 chars, GBA encoding).</td></tr>
    <tr><td>POST /api/slot/:i/change_held_item</td><td><code>{"party_position":&lt;u8 0–5&gt;,"item_id":&lt;u16&gt;}</code></td><td>Set held item. <code>item_id=0</code> removes it.</td></tr>
    <tr><td>POST /api/slot/:i/cure_status</td><td><code>{"party_position":&lt;u8 0–5&gt;}</code></td><td>Zero the 4-byte status word (clears all status conditions).</td></tr>
    <tr><td>POST /api/slot/:i/restore_hp</td><td><code>{"party_position":&lt;u8 0–5&gt;}</code></td><td>Write max HP to current HP.</td></tr>
    <tr><td>POST /api/slot/:i/restore_pp</td><td><code>{"party_position":&lt;u8 0–5&gt;}</code></td><td>Restore all four move PP to current maximum.</td></tr>
    <tr><td>POST /api/slot/:i/set_friendship</td><td><code>{"party_position":&lt;u8 0–5&gt;,"friendship":&lt;u8 0–255&gt;}</code></td><td>Set the friendship byte.</td></tr>
    <tr><td>POST /api/slot/:i/change_move</td><td><code>{"party_position":&lt;u8 0–5&gt;,"slot":&lt;u8 0–3&gt;,"move_id":&lt;u16&gt;}</code></td><td>Replace a move slot. <code>move_id=0</code> clears the slot.</td></tr>
    <tr><td>POST /api/slot/:i/set_ivs</td><td><code>{"party_position":&lt;u8 0–5&gt;,"hp":…,"atk":…,"def":…,"spd":…,"spa":…,"spdef":…}</code></td><td>Set all six IVs (each clamped to 31).</td></tr>
    <tr><td>POST /api/slot/:i/increase_ivs</td><td><code>{"party_position":&lt;u8 0–5&gt;,"hp":…,"atk":…,"def":…,"spd":…,"spa":…,"spdef":…}</code></td><td>Add to each IV; each stat is clamped at 31.</td></tr>
    <tr><td>POST /api/slot/:i/set_evs</td><td><code>{"party_position":&lt;u8 0–5&gt;,"hp":…,"atk":…,"def":…,"spd":…,"spa":…,"spdef":…}</code></td><td>Set all six EVs (0–255 each).</td></tr>
    <tr><td>POST /api/slot/:i/increase_evs</td><td><code>{"party_position":&lt;u8 0–5&gt;,"hp":…,"atk":…,"def":…,"spd":…,"spa":…,"spdef":…}</code></td><td>Add to each EV; each stat is clamped at 255.</td></tr>
    <tr><td>POST /api/slot/:i/set_exp</td><td><code>{"party_position":&lt;u8 0–5&gt;,"exp":&lt;u32&gt;}</code></td><td>Write raw experience points into the Growth substructure. Level byte is not changed.</td></tr>
    <tr><td>POST /api/slot/:i/set_level</td><td><code>{"party_position":&lt;u8 0–5&gt;,"level":&lt;u8 1–100&gt;}</code></td><td>Write level byte and set experience to the Gen III minimum for that level.</td></tr>
    <tr><td>POST /api/slot/:i/learn_move</td><td><code>{"party_position":&lt;u8 0–5&gt;,"move_id":&lt;u16&gt;}</code></td><td>Write the move into the first empty move slot. No-op if the Pokémon already knows it or all four slots are full.</td></tr>
    <tr><td>POST /api/slot/:i/forget_move</td><td><code>{"party_position":&lt;u8 0–5&gt;,"slot":&lt;u8 0–3&gt;}</code></td><td>Clear a move slot and compact remaining moves upward.</td></tr>
    <tr><td>POST /api/slot/:i/set_pokerus</td><td><code>{"party_position":&lt;u8 0–5&gt;}</code></td><td>Write a Pokérus infection byte (strain 1, 4 days remaining).</td></tr>
    <tr><td>POST /api/slot/:i/set_pp_ups</td><td><code>{"party_position":&lt;u8 0–5&gt;,"pp0":…,"pp1":…,"pp2":…,"pp3":…}</code></td><td>Set PP-Up bonus for all four moves (0–3 each).</td></tr>
    <tr><td>POST /api/slot/:i/revive_pokemon</td><td><code>{"party_position":&lt;u8 0–5&gt;,"personality":&lt;u32&gt;}</code></td><td>Look up a dead Pokémon in the run's graveyard by its personality value and write it into the party slot (HP=1, status=0). Get the personality from <code>GET /api/run/:id</code> → <code>dead[].personality</code>.</td></tr>
    <tr><td>POST /api/slot/:i/heal_party</td><td><em>no body</em></td><td>Restore HP and cure status for all six party slots in one pass.</td></tr>
    <tr><td>POST /api/slot/:i/undo</td><td><em>no body</em></td><td>Revert the most recent injection by replaying pre-write EWRAM bytes.</td></tr>
  </tbody>
</table>

<h3>Batch &amp; Presets</h3>
<table>
  <thead><tr><th>Method &amp; Path</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>POST /api/batch</td><td>Run multiple injection commands across any slots in one request. See <a href="#injections">Using the Injection API</a>.</td></tr>
    <tr><td>GET /api/presets</td><td>List saved injection presets.</td></tr>
    <tr><td>POST /api/presets</td><td>Save a new preset.</td></tr>
    <tr><td>DELETE /api/presets/:id</td><td>Delete a preset.</td></tr>
  </tbody>
</table>

<!-- ── Injection API deep-dive ───────────────────────────────────────────── -->
<h2 id="injections">Using the Injection API</h2>
<p>Injection commands write directly to the GBA's EWRAM via RetroArch's <code>WRITE_CORE_MEMORY</code> network command. They are <strong>asynchronous</strong>: each endpoint validates the request and enqueues the write, returning <code>200</code> immediately. The write is applied on the next poll cycle (~100 ms). Do not send a second command until the first has taken effect if ordering matters.</p>
<div class="warn">Injections are only available while a slot is connected and <code>allow_injections = true</code> in config (the default). All injection endpoints return <code>403</code> when injections are disabled.</div>

<h3>Getting a token for scripts</h3>
<pre><code># Step 1 — log in and save the cookie jar
curl -s -c cookies.txt -X POST http://localhost:9090/api/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"alice","password":"secret"}'

# Step 2 — retrieve the raw token value
TOKEN=$(curl -s -b cookies.txt http://localhost:9090/api/me/token | jq -r .token)

# Step 3 — use it on injection calls
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/heal_party</code></pre>

<h3>Give and take items</h3>
<p>Items are identified by their Generation III internal item ID. Use <code>GET /api/slot/:index/bag</code> to see the IDs of items currently in the bag, or consult the table below for common items.</p>
<pre><code># Give 5 Rare Candies to slot 0
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/give_item \
  -H 'Content-Type: application/json' \
  -d '{"item_id": 44, "quantity": 5}'

# Take 10 Poké Balls from slot 0
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/take_item \
  -H 'Content-Type: application/json' \
  -d '{"item_id": 4, "quantity": 10}'</code></pre>

<p><strong>give_item</strong> always adds to the <em>items pocket</em> — it cannot add to key items, Pokéballs, or TMs. <strong>take_item</strong> searches all pockets. Quantity must be 1–99 for both; <code>give_item</code> will reject a request if there is no room left in the pocket (max 30 unique item types).</p>

<h3>Common item IDs (FireRed/LeafGreen)</h3>
<table>
  <thead><tr><th>ID</th><th>Item</th><th>ID</th><th>Item</th></tr></thead>
  <tbody>
    <tr><td>1</td><td>Master Ball</td><td>24</td><td>Revive</td></tr>
    <tr><td>2</td><td>Ultra Ball</td><td>25</td><td>Max Revive</td></tr>
    <tr><td>3</td><td>Great Ball</td><td>34</td><td>Ether</td></tr>
    <tr><td>4</td><td>Poké Ball</td><td>35</td><td>Max Ether</td></tr>
    <tr><td>6</td><td>Net Ball</td><td>36</td><td>Elixir</td></tr>
    <tr><td>8</td><td>Nest Ball</td><td>37</td><td>Max Elixir</td></tr>
    <tr><td>9</td><td>Repeat Ball</td><td>44</td><td>Rare Candy</td></tr>
    <tr><td>10</td><td>Timer Ball</td><td>45</td><td>PP Up</td></tr>
    <tr><td>13</td><td>Potion</td><td>46</td><td>Zinc</td></tr>
    <tr><td>19</td><td>Full Restore</td><td>47</td><td>Carbos</td></tr>
    <tr><td>20</td><td>Max Potion</td><td>48</td><td>Calcium</td></tr>
    <tr><td>21</td><td>Hyper Potion</td><td>49</td><td>Protein</td></tr>
    <tr><td>22</td><td>Super Potion</td><td>50</td><td>Iron</td></tr>
    <tr><td>23</td><td>Full Heal</td><td>51</td><td>HP Up</td></tr>
  </tbody>
</table>
<div class="note">Item IDs are read from your ROM at startup, so they are correct for vanilla FireRed/LeafGreen and most ROM hacks that keep the standard item table. Use <code>GET /api/slot/:index/bag</code> to confirm IDs for any item already in the bag.</div>

<h3>Modifying a Pokémon</h3>
<pre><code># Set slot 0, party position 0 (lead) to level 50
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/set_level \
  -H 'Content-Type: application/json' \
  -d '{"party_position": 0, "level": 50}'

# Give the lead Pokémon max IVs in all stats
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/set_ivs \
  -H 'Content-Type: application/json' \
  -d '{"party_position": 0, "hp": 31, "atk": 31, "def": 31, "spd": 31, "spa": 31, "spdef": 31}'

# Teach the lead Pokémon Surf (move_id 57)
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/learn_move \
  -H 'Content-Type: application/json' \
  -d '{"party_position": 0, "move_id": 57}'</code></pre>

<h3>Reviving a dead Pokémon</h3>
<p>First, get the dead Pokémon's <code>personality</code> value from the run's graveyard:</p>
<pre><code>curl -s -H "Authorization: Bearer $TOKEN" http://localhost:9090/api/run/1 | jq '.dead[] | {nickname, personality}'</code></pre>
<p>Then revive it into a party slot (e.g. slot 2 in the party):</p>
<pre><code>curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/revive_pokemon \
  -H 'Content-Type: application/json' \
  -d '{"party_position": 2, "personality": 3141592653}'</code></pre>

<h3>Undoing a command</h3>
<p>Each slot stores a snapshot of the EWRAM bytes that were about to be overwritten by the last injection. To restore them:</p>
<pre><code>curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/slot/0/undo</code></pre>
<p>Undo only covers the immediately preceding command. Calling undo twice in a row undoes the undo.</p>

<h3>Batch injection</h3>
<p><code>POST /api/batch</code> accepts a JSON array of <code>{"slot": &lt;index&gt;, "message": &lt;ClientMessage&gt;}</code> objects. All items are validated before any command is enqueued — if one entry fails, none are applied.</p>
<p>The <code>message</code> field uses serde's <strong>externally tagged enum</strong> format: a single-key object where the key is the variant name and the value is the fields object (or <code>null</code> for fieldless variants like <code>HealParty</code>).</p>
<pre><code># Give 3 Rare Candies to slot 0 AND heal slot 1's party, atomically
curl -s -H "Authorization: Bearer $TOKEN" \
  -X POST http://localhost:9090/api/batch \
  -H 'Content-Type: application/json' \
  -d '[
    {"slot": 0, "message": {"GiveItem": {"item_id": 44, "quantity": 3}}},
    {"slot": 1, "message": {"HealParty": null}},
    {"slot": 0, "message": {"SetLevel": {"party_position": 0, "level": 50}}}
  ]'
# → {"queued": 3}</code></pre>

<h4>ClientMessage variant names for batch</h4>
<table>
  <thead><tr><th>Variant (key)</th><th>Fields</th></tr></thead>
  <tbody>
    <tr><td>GiveItem</td><td><code>item_id</code>, <code>quantity</code></td></tr>
    <tr><td>TakeItem</td><td><code>item_id</code>, <code>quantity</code></td></tr>
    <tr><td>MakeShiny</td><td><code>party_position</code></td></tr>
    <tr><td>ChangeSpecies</td><td><code>party_position</code>, <code>new_species</code></td></tr>
    <tr><td>ChangeAbility</td><td><code>party_position</code>, <code>ability_slot</code></td></tr>
    <tr><td>ChangeGender</td><td><code>party_position</code>, <code>target_gender</code></td></tr>
    <tr><td>ChangeNickname</td><td><code>party_position</code>, <code>nickname</code></td></tr>
    <tr><td>ChangeHeldItem</td><td><code>party_position</code>, <code>item_id</code></td></tr>
    <tr><td>CureStatus</td><td><code>party_position</code></td></tr>
    <tr><td>ChangeNature</td><td><code>party_position</code>, <code>target_nature</code></td></tr>
    <tr><td>RestorePp</td><td><code>party_position</code></td></tr>
    <tr><td>SetFriendship</td><td><code>party_position</code>, <code>friendship</code></td></tr>
    <tr><td>ChangeMove</td><td><code>party_position</code>, <code>slot</code>, <code>move_id</code></td></tr>
    <tr><td>SetIvs</td><td><code>party_position</code>, <code>hp</code>, <code>atk</code>, <code>def</code>, <code>spd</code>, <code>spa</code>, <code>spdef</code></td></tr>
    <tr><td>IncreaseIvs</td><td><code>party_position</code>, <code>hp</code>, <code>atk</code>, <code>def</code>, <code>spd</code>, <code>spa</code>, <code>spdef</code></td></tr>
    <tr><td>SetEvs</td><td><code>party_position</code>, <code>hp</code>, <code>atk</code>, <code>def</code>, <code>spd</code>, <code>spa</code>, <code>spdef</code></td></tr>
    <tr><td>IncreaseEvs</td><td><code>party_position</code>, <code>hp</code>, <code>atk</code>, <code>def</code>, <code>spd</code>, <code>spa</code>, <code>spdef</code></td></tr>
    <tr><td>RestoreHp</td><td><code>party_position</code></td></tr>
    <tr><td>HealParty</td><td><em>null</em></td></tr>
    <tr><td>SetExp</td><td><code>party_position</code>, <code>exp</code></td></tr>
    <tr><td>SetLevel</td><td><code>party_position</code>, <code>level</code></td></tr>
    <tr><td>LearnMove</td><td><code>party_position</code>, <code>move_id</code></td></tr>
    <tr><td>ForgetMove</td><td><code>party_position</code>, <code>slot</code></td></tr>
    <tr><td>SetPokerus</td><td><code>party_position</code></td></tr>
    <tr><td>SetPpUps</td><td><code>party_position</code>, <code>pp0</code>, <code>pp1</code>, <code>pp2</code>, <code>pp3</code></td></tr>
    <tr><td>RevivePokemon</td><td><code>party_position</code>, <code>personality</code></td></tr>
    <tr><td>UndoLastCommand</td><td><em>null</em></td></tr>
  </tbody>
</table>

<!-- ── WebSocket Overlay ───────────────────────────────────────────────── -->
<h2 id="websocket">WebSocket Overlay</h2>
<p>The aggregator pushes live state updates over WebSocket. Connect to:</p>
<pre><code>ws://&lt;host&gt;:9090/ws</code></pre>
<p>Every message is a JSON object with a <code>type</code> field. The main types are <code>state</code> (full slot snapshot) and <code>event</code> (death, catch, shiny, badge, game_cleared).</p>
<div class="note">The built-in overlay pages (<code>/0/party</code>, <code>/0/routes</code>, etc.) connect to this WebSocket automatically — you don't need to handle it yourself unless building a custom overlay.</div>

<h3>OBS Browser Source setup</h3>
<ol>
  <li>In OBS, add a <strong>Browser Source</strong>.</li>
  <li>Set the URL to an overlay page, e.g. <code>http://localhost:9090/0/party?token=YOUR_TOKEN</code>.</li>
  <li>Set width/height to match the overlay's layout (party: 600×120, routes: 400×600, etc.).</li>
  <li>Check <strong>Shutdown source when not visible</strong> to avoid background CPU use.</li>
</ol>

<h3>Available browser source pages</h3>
<table>
  <thead><tr><th>URL</th><th>Content</th></tr></thead>
  <tbody>
    <tr><td>/:index/party</td><td>Live party (sprites, levels, HP)</td></tr>
    <tr><td>/:index/items</td><td>Bag item viewer — all four pockets with item names and quantities</td></tr>
    <tr><td>/:index/routes</td><td>Route encounter table with caught/dead markers</td></tr>
    <tr><td>/:index/encounters</td><td>Focused encounter list</td></tr>
    <tr><td>/:index/caught</td><td>All caught Pokémon</td></tr>
    <tr><td>/:index/dead</td><td>All dead Pokémon</td></tr>
    <tr><td>/:index/hp</td><td>HP bars for current party</td></tr>
    <tr><td>/:index/badges</td><td>Earned badge dot-bar</td></tr>
    <tr><td>/:index/deaths</td><td>Death counter</td></tr>
    <tr><td>/:index/encounter_count</td><td>Encounter counter for current area</td></tr>
    <tr><td>/:index/nextgym</td><td>Type-advantage panel for next gym</td></tr>
    <tr><td>/:index/money</td><td>Current money</td></tr>
    <tr><td>/:index/playtime</td><td>In-game playtime</td></tr>
    <tr><td>/:index/goals</td><td>Run goals progress</td></tr>
    <tr><td>/:index/types</td><td>Type coverage for current party</td></tr>
    <tr><td>/:index/moves</td><td>Move usage stats</td></tr>
    <tr><td>/:index/vs_leader</td><td>Type matchup vs next gym leader</td></tr>
    <tr><td>/:index/encounter_table</td><td>Full area encounter table</td></tr>
    <tr><td>/alerts</td><td>Toast notifications (death, shiny, badge)</td></tr>
  </tbody>
</table>
<p><code>:index</code> is the slot number (0-based). In single-player mode it is always <code>0</code>.</p>

<!-- ── OBS Integration ────────────────────────────────────────────────── -->
<h2 id="obs">OBS Integration</h2>
<p>The tracker can trigger OBS <strong>replay buffer saves</strong> and <strong>scene switches</strong> automatically on game events. Requires OBS 28+ with the WebSocket server plugin enabled (built-in since OBS 28).</p>

<h3>Enable in OBS</h3>
<ol>
  <li>In OBS: <strong>Tools → WebSocket Server Settings</strong>.</li>
  <li>Check <strong>Enable WebSocket server</strong>. Note the port (default <code>4455</code>) and set a password if desired.</li>
  <li>Enable the <strong>Replay Buffer</strong> under <strong>Output → Replay Buffer</strong> for clip triggers to work.</li>
</ol>

<h3>Config</h3>
<p>Add an <code>[obs]</code> section to your config file, or configure it per-user at <a href="/integrations">/integrations</a>:</p>
<pre><code>[obs]
host     = "localhost"   # OBS machine hostname or IP
port     = 4455          # OBS WebSocket port
password = "secret"      # leave blank if auth is disabled

# Replay buffer save triggers
clip_on_death = true
clip_on_shiny = true
clip_on_wipe  = false
clip_on_badge = false

# Scene switch triggers (set to a scene name, or omit to disable)
scene_on_death = "Death Cam"
scene_on_shiny = "Shiny Cam"
scene_on_wipe  = "End Screen"
scene_on_badge = "Badge Cam"
scene_on_catch = "Catch Cam"</code></pre>
<div class="note">Per-user OBS configs set via <a href="/integrations">/integrations</a> take precedence over the global <code>[obs]</code> section for that user's direct-mode slots.</div>

<!-- ── Twitch Bot ─────────────────────────────────────────────────────── -->
<h2 id="twitch">Twitch Bot</h2>
<p>The aggregator can join a Twitch channel and respond to chat commands. It can also listen for Channel Points redemptions via EventSub.</p>

<h3>Config</h3>
<pre><code>[twitch]
channel      = "yourchannel"        # channel name without #
nick         = "your_bot_account"   # Twitch username of the bot
token        = "oauth:xxxxxxxxxx"   # get one at twitchapps.com/tmi
slot         = 0                    # tracker slot index to read from

# Optional: Channel Points EventSub
client_id      = "abc123"           # from dev.twitch.tv
broadcaster_id = "12345678"         # numeric Twitch user ID of the channel</code></pre>

<h3>Chat commands</h3>
<table>
  <thead><tr><th>Command</th><th>Response</th></tr></thead>
  <tbody>
    <tr><td>!party</td><td>Current party members — nickname, species, level, HP.</td></tr>
    <tr><td>!deaths</td><td>Death count and the five most recent deaths (requires DB).</td></tr>
    <tr><td>!shinies</td><td>Shiny count and last shiny name (requires DB).</td></tr>
    <tr><td>!status</td><td>One-liner: <code>Player — HP/MaxHP — Zone</code></td></tr>
    <tr><td>!moves</td><td>Lead Pokémon's move set with current PP for each slot.</td></tr>
    <tr><td>!ivs</td><td>Lead Pokémon's IVs for all six stats.</td></tr>
    <tr><td>!badges</td><td>Badge count and names earned so far.</td></tr>
    <tr><td>!bag</td><td>Items and Pokéballs currently in the player's bag.</td></tr>
    <tr><td>!map</td><td>Player's current map / location name.</td></tr>
    <tr><td>!encounter</td><td>Current route's encounter table — species and percentage rates.</td></tr>
    <tr><td>!luck</td><td>Shiny count / total encounters with expected-rate comparison (requires DB).</td></tr>
    <tr><td>!timer</td><td>Elapsed HH:MM:SS from run start (requires DB).</td></tr>
    <tr><td>!box</td><td>PC box Pokémon count with species list (requires DB).</td></tr>
    <tr><td>!run</td><td>Current run ID, caught count, and death count (requires DB).</td></tr>
  </tbody>
</table>

<h3>Channel Points (EventSub)</h3>
<p>Map reward UUIDs to aggregator commands in your config:</p>
<pre><code>[twitch.reward_commands]
"00000000-0000-0000-0000-000000000001" = "heal_all"
"00000000-0000-0000-0000-000000000002" = "new_run"</code></pre>
<p>Supported commands: <code>heal_all</code>, <code>new_run</code>, <code>end_run</code>.</p>
<p>Get reward UUIDs from the Twitch API or by inspecting EventSub payloads. The <code>client_id</code> and <code>broadcaster_id</code> fields are required for EventSub to work.</p>
<div class="note">Each user can configure their own Twitch bot independently via <a href="/integrations">/integrations</a> without touching the shared config file.</div>

<!-- ── YouTube Chat Bot ───────────────────────────────────────────────── -->
<h2 id="youtube">YouTube Chat Bot</h2>
<p>Polls the YouTube Live chat API and responds to the same commands as the Twitch bot.</p>

<h3>Config</h3>
<pre><code>[youtube_chat]
api_key      = "AIzaSy..."           # YouTube Data API v3 key
broadcast_id = "dQw4w9WgXcQ"        # live broadcast ID from the stream URL
slot         = 0                     # tracker slot to read from
poll_secs    = 15                    # polling interval (min 5)</code></pre>

<h3>Getting a YouTube API key</h3>
<ol>
  <li>Go to <strong>console.cloud.google.com</strong> and create a project.</li>
  <li>Enable the <strong>YouTube Data API v3</strong>.</li>
  <li>Create an API key under <strong>Credentials</strong> and restrict it to YouTube Data API v3.</li>
</ol>
<p>The broadcast ID is the <code>v=</code> value in your live stream URL. Find it in YouTube Studio under the live stream details.</p>
<div class="note">YouTube API has a daily quota. A poll interval of 15 s uses roughly 5 000–10 000 units/day out of a 10 000-unit default quota.</div>

<!-- ── Discord ────────────────────────────────────────────────────────── -->
<h2 id="discord">Discord</h2>
<p>Three independent Discord integrations are available: a live embed, a run thread, and slash commands.</p>

<h3>Live Status Embed</h3>
<p>Keeps a pinned message in a channel updated with live party and run info.</p>
<pre><code>[discord_live_embed]
bot_token            = "Bot MTc…"
channel_id           = 123456789012345678   # channel to post in
message_id           = 987654321098765432   # ID of the message to edit
update_interval_secs = 30                   # how often to refresh (min 10)</code></pre>
<p>Create the initial message manually in the channel, then copy its ID (enable Developer Mode in Discord settings to see IDs).</p>

<h3>Run Thread</h3>
<p>Creates a new thread in a channel at run start and posts milestone messages (badge earned, death, shiny, game cleared).</p>
<pre><code>[discord_run_thread]
bot_token  = "Bot MTc…"
channel_id = 123456789012345678</code></pre>

<h3>Slash Commands</h3>
<p>Registers Application Commands with Discord so users can query run state from any channel.</p>
<pre><code>[discord_slash]
app_id     = 123456789012345678       # Application ID from Discord dev portal
public_key = "abc123def456…"          # Ed25519 public key (hex)
token      = "Bot MTc…"               # Bot token
guild_id   = 987654321098765432       # omit for global commands</code></pre>
<p>Set your interactions endpoint URL in the Discord dev portal to <code>https://&lt;your-aggregator&gt;/interactions</code>. The aggregator handles verification and command dispatch automatically.</p>
<div class="note">Each user can run their own Discord bot instance independently via <a href="/integrations">/integrations</a>.</div>

<!-- ── Webhooks ───────────────────────────────────────────────────────── -->
<h2 id="webhooks">Webhooks</h2>
<p>POST a JSON payload to a URL of your choice when game events occur. Compatible with Discord webhooks, custom HTTP endpoints, and stream alert services.</p>

<h3>Config</h3>
<pre><code>[webhooks]
death_url   = "https://discord.com/api/webhooks/…"
catch_url   = "https://your-server.example.com/hook"
shiny_url   = "https://discord.com/api/webhooks/…"
wipe_url    = ""
badge_url   = ""
nickname_url = ""

# Optional: custom JSON body template.
# Omit to use the default structured payload.
death_template = "{\"content\":\"{pokemon.nickname} ({pokemon.species}) fainted at Lv {pokemon.level}!\"}"

# Optional: HMAC-SHA256 request signing
hmac_secret = "your-shared-secret"

# Discord rich embed (uses Discord's embed format)
discord_webhook_url = "https://discord.com/api/webhooks/…"</code></pre>

<h3>Template variables</h3>
<p>Use single braces: <code>{player}</code>, <code>{pokemon.nickname}</code>, etc. Unknown placeholders are left unchanged.</p>
<table>
  <thead><tr><th>Variable</th><th>Value</th><th>Notes</th></tr></thead>
  <tbody>
    <tr><td>{event}</td><td><code>death</code>, <code>catch</code>, <code>shiny</code>, <code>wipe</code>, <code>badge</code></td><td></td></tr>
    <tr><td>{player}</td><td>Player name from config</td><td></td></tr>
    <tr><td>{timestamp}</td><td>Unix timestamp in seconds</td><td></td></tr>
    <tr><td>{pokemon.nickname}</td><td>Pokémon's in-game nickname</td><td>Empty string for <code>wipe</code></td></tr>
    <tr><td>{pokemon.species}</td><td>Species name</td><td>Empty string for <code>wipe</code></td></tr>
    <tr><td>{pokemon.level}</td><td>Level as a plain integer string</td><td>Empty string for <code>wipe</code></td></tr>
    <tr><td>{pokemon.shiny}</td><td><code>true</code> or <code>false</code></td><td>Empty string for <code>wipe</code></td></tr>
    <tr><td>{pokemon.nature}</td><td>Nature name</td><td>Empty string for <code>wipe</code></td></tr>
    <tr><td>{badge.name}</td><td>Badge name (e.g. <code>Boulder Badge</code>)</td><td>Only meaningful for <code>badge</code> events</td></tr>
    <tr><td>{pokemon.old_name}</td><td>Previous nickname before rename</td><td>Only meaningful for <code>nickname_change</code></td></tr>
    <tr><td>{pokemon.new_name}</td><td>New nickname after rename</td><td>Only meaningful for <code>nickname_change</code></td></tr>
  </tbody>
</table>

<h3>HMAC signing</h3>
<p>When <code>hmac_secret</code> is set, every webhook request includes an <code>X-Hub-Signature-256</code> header containing <code>sha256=&lt;hex&gt;</code>, computed over the raw request body. Verify it on your server to confirm the request is genuine.</p>

<!-- ── Per-User Integrations ──────────────────────────────────────────── -->
<h2 id="per-user">Per-User Integrations</h2>
<p>Each user account can run its own independent Twitch bot, YouTube bot, Discord embed, Discord run thread, and OBS connection — completely separate from the shared global config and from other users.</p>
<p>Manage them at <a href="/integrations">/integrations</a> once logged in.</p>
<p>How it works:</p>
<ul>
  <li>Configs are stored in the database under <code>user_integrations</code>.</li>
  <li>When you save a config, the old thread (if any) is stopped and a new one is started using the new settings.</li>
  <li>Per-user threads filter slot data to only the runs accessible to that user.</li>
  <li>Deleting an integration config stops its thread immediately.</li>
</ul>
<div class="note">Per-user OBS configs are applied to your direct-mode slots. The global <code>[obs]</code> config applies to all other slots.</div>

<!-- ── Config Reference ───────────────────────────────────────────────── -->
<h2 id="config">Config File Reference</h2>
<p>The config file lives at <code>~/.config/fire_red_aggregator/config.toml</code> by default. Edit it with <code>--config-editor</code> (GUI) or <code>--config-editor-cli</code> (terminal).</p>

<h3>Core settings</h3>
<table>
  <thead><tr><th>Key</th><th>Default</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>ws_port</td><td>—</td><td>WebSocket overlay port. Set to enable headless/web mode.</td></tr>
    <tr><td>db</td><td>—</td><td>PostgreSQL connection string, e.g. <code>postgresql://localhost/nuzlocke</code>.</td></tr>
    <tr><td>allow_injections</td><td>true</td><td>Enable injection API endpoints.</td></tr>
    <tr><td>direct_mode</td><td>false</td><td>Enable /join page for on-demand connections.</td></tr>
    <tr><td>backup_dir</td><td>—</td><td>Directory for automatic run JSON backups on game clear.</td></tr>
    <tr><td>poll_ms</td><td>100</td><td>Game memory poll interval in ms (direct mode).</td></tr>
    <tr><td>rom_path</td><td>—</td><td>Path to the FireRed ROM (required for direct mode).</td></tr>
    <tr><td>retroarch_hosts</td><td>[]</td><td>List of RetroArch host IPs to poll directly.</td></tr>
    <tr><td>retroarch_port</td><td>55355</td><td>RetroArch network-commands UDP port.</td></tr>
    <tr><td>dupes_clause</td><td>off</td><td>Duplication clause mode: <code>off</code>, <code>per_player</code>, <code>shared</code>.</td></tr>
    <tr><td>allow_species_repeats</td><td>false</td><td>Randomizer mode — allow same species on multiple routes.</td></tr>
    <tr><td>run_start_balls</td><td>—</td><td>Minimum Pokéballs before run tracking starts.</td></tr>
    <tr><td>livesplit_host</td><td>—</td><td>LiveSplit Server hostname. Enables split bridge.</td></tr>
    <tr><td>livesplit_port</td><td>16834</td><td>LiveSplit Server TCP port.</td></tr>
    <tr><td>livesplit_split_on_badges</td><td>false</td><td>Fire a split every time a badge is earned.</td></tr>
    <tr><td>default_test</td><td>false</td><td>Always run in test mode.</td></tr>
  </tbody>
</table>

<h3>Integration sections</h3>
<table>
  <thead><tr><th>Section</th><th>Description</th></tr></thead>
  <tbody>
    <tr><td>[twitch]</td><td>Twitch IRC bot. See <a href="#twitch">Twitch</a>.</td></tr>
    <tr><td>[youtube_chat]</td><td>YouTube Live chat bot. See <a href="#youtube">YouTube</a>.</td></tr>
    <tr><td>[discord_live_embed]</td><td>Live status embed. See <a href="#discord">Discord</a>.</td></tr>
    <tr><td>[discord_run_thread]</td><td>Per-run milestone threads. See <a href="#discord">Discord</a>.</td></tr>
    <tr><td>[discord_slash]</td><td>Slash commands endpoint. See <a href="#discord">Discord</a>.</td></tr>
    <tr><td>[obs]</td><td>OBS clip and scene triggers. See <a href="#obs">OBS</a>.</td></tr>
    <tr><td>[webhooks]</td><td>Per-event HTTP webhooks. See <a href="#webhooks">Webhooks</a>.</td></tr>
    <tr><td>[test]</td><td>Override settings when <code>--test</code> is active.</td></tr>
  </tbody>
</table>
</div>
</body>
</html>"##;

pub(crate) async fn serve_guide_page(
    State(state): State<WebState>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        apply_page(GUIDE_HTML, state.testing),
    )
}

/// `GET /api/me/dashboard` — full dashboard JSON for the authenticated user.
pub(crate) async fn api_me_dashboard(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let live_slots = state.live_slots.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        let mut data = fire_red_database::user_dashboard_json(&conn, user.id);

        // Find which live slot is running this user's first open run.
        let my_run_id: Option<u32> = data["open_runs"]
            .as_array()
            .and_then(|arr| arr.iter().find(|r| r["is_owner"].as_bool() == Some(true)))
            .and_then(|r| r["id"].as_i64())
            .map(|id| id as u32);

        if let Some(run_id) = my_run_id {
            let slots = live_slots.lock_or_recover();
            let my_slot = slots.iter().position(|s| {
                s.db.as_ref().and_then(|db| db.active_run_id()) == Some(run_id)
            });
            if let Some(obj) = data.as_object_mut() {
                obj.insert("my_slot".to_string(), serde_json::json!(my_slot));
            }
        } else if let Some(obj) = data.as_object_mut() {
            obj.insert("my_slot".to_string(), serde_json::Value::Null);
        }

        Ok(data)
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct InviteBody { username: String }

/// `POST /api/run/:id/invite` — invite a user (by username) to a run.
pub(crate) const MOBILE_APP_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0,maximum-scale=1.0,user-scalable=no">
<meta name="mobile-web-app-capable" content="yes">
<meta name="apple-mobile-web-app-capable" content="yes">
<title>Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0;-webkit-tap-highlight-color:transparent}
html,body{height:100%;overflow:hidden}
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:#1a1a2e;color:#eee;display:flex;flex-direction:column}
.app-header{background:#0d1b30;border-bottom:1px solid #0f3460;padding:.75rem 1rem;display:flex;align-items:center;justify-content:space-between;flex-shrink:0}
.app-title{font-size:1.1rem;font-weight:700;color:#e94560}
.app-subtitle{font-size:.72rem;color:#888;margin-top:.1rem}
.tab-content{flex:1;overflow-y:auto;-webkit-overflow-scrolling:touch;padding:1rem;padding-bottom:80px}
.tab-panel{display:none}
.tab-panel.active{display:block}
.bottom-nav{position:fixed;bottom:0;left:0;right:0;background:#0d1b30;border-top:1px solid #0f3460;display:flex;height:60px;flex-shrink:0;z-index:100}
.nav-btn{flex:1;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:3px;border:none;background:none;color:#555;font-size:.62rem;cursor:pointer;transition:color .12s;padding:0}
.nav-btn.active{color:#e94560}
.nav-btn svg{width:20px;height:20px;flex-shrink:0}
.card{background:#16213e;border:1px solid #0f3460;border-radius:10px;padding:1rem;margin-bottom:.75rem}
.card-title{font-size:.68rem;font-weight:700;text-transform:uppercase;letter-spacing:.6px;color:#555;margin-bottom:.75rem}
.run-header{display:flex;align-items:center;gap:.75rem;margin-bottom:.9rem}
.run-badge{background:#e94560;color:#fff;border-radius:6px;padding:.28rem .55rem;font-size:.8rem;font-weight:700;flex-shrink:0}
.run-player{font-size:1rem;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.run-active{font-size:.72rem;color:#7dce7d}
.stat-row{display:grid;grid-template-columns:repeat(3,1fr);gap:.5rem;margin-bottom:.9rem}
.stat-box{background:#0f3460;border-radius:8px;padding:.6rem;text-align:center}
.stat-num{font-size:1.35rem;font-weight:700;line-height:1}
.stat-num.red{color:#e94560}.stat-num.green{color:#7dce7d}.stat-num.blue{color:#5090e0}
.stat-lbl{font-size:.6rem;color:#888;margin-top:.18rem;text-transform:uppercase;letter-spacing:.4px}
.party-grid{display:grid;grid-template-columns:1fr 1fr;gap:.5rem}
.party-mon{background:#0f3460;border-radius:8px;padding:.7rem}
.mon-name{font-size:.88rem;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.mon-species{font-size:.73rem;color:#888;margin-top:.1rem}
.mon-level{font-size:.73rem;color:#5090e0;margin-top:.1rem}
.mon-hp-bar{height:4px;border-radius:2px;background:#1e3a6e;margin-top:.35rem;overflow:hidden}
.mon-hp-fill{height:100%;border-radius:2px;transition:width .3s}
.shiny-star{color:#f0d060}
.run-row{background:#16213e;border:1px solid #0f3460;border-radius:8px;padding:.8rem;margin-bottom:.5rem;display:flex;align-items:center;gap:.6rem}
.run-row-info{flex:1;min-width:0}
.run-row-id{font-size:.75rem;font-weight:700;color:#5090e0}
.run-row-player{font-size:.9rem;font-weight:600;margin-top:.08rem;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.run-row-meta{font-size:.72rem;color:#888;margin-top:.18rem}
.run-row-actions{display:flex;flex-direction:column;gap:.28rem;flex-shrink:0}
.btn{display:inline-flex;align-items:center;justify-content:center;padding:.45rem .9rem;border:none;border-radius:6px;font-size:.82rem;cursor:pointer;text-decoration:none;font-family:inherit;font-weight:500;white-space:nowrap}
.btn-primary{background:#e94560;color:#fff}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-danger{background:#5c1a1a;color:#ce7d7d;border:1px solid #8a2d2d}
.btn-sm{padding:.32rem .6rem;font-size:.75rem}
.btn-block{width:100%;text-align:center;margin-bottom:.4rem}
.btn:active{opacity:.75}
.setting-row{display:flex;align-items:center;justify-content:space-between;padding:.85rem 0;border-bottom:1px solid rgba(255,255,255,0.06)}
.setting-row:last-child{border-bottom:none}
.setting-label{font-size:.88rem;font-weight:500}
.setting-desc{font-size:.72rem;color:#666;margin-top:.12rem}
.links-grid{display:grid;grid-template-columns:1fr 1fr;gap:.4rem;margin-top:.25rem}
.empty{color:#666;font-size:.85rem;text-align:center;padding:2rem 0}
.loading{color:#888;font-size:.85rem;text-align:center;padding:2rem 0}
a{color:#5090e0;text-decoration:none}
</style>
</head>
<body>
<div class="app-header">
  <div>
    <div class="app-title">🔴 Fire Red Tracker</div>
    <div class="app-subtitle" id="header-sub">Loading…</div>
  </div>
</div>

<div class="tab-content">

  <!-- ── Home ──────────────────────────────────────────────── -->
  <div id="tab-home" class="tab-panel active">
    <div id="home-loading" class="loading">Loading…</div>
    <div id="home-content" style="display:none">
      <div class="card">
        <div class="card-title">Overview</div>
        <div class="stat-row">
          <div class="stat-box"><div class="stat-num blue" id="stat-runs">—</div><div class="stat-lbl">Runs</div></div>
          <div class="stat-box"><div class="stat-num green" id="stat-catches">—</div><div class="stat-lbl">Caught</div></div>
          <div class="stat-box"><div class="stat-num red" id="stat-deaths">—</div><div class="stat-lbl">Deaths</div></div>
        </div>
        <div class="run-header" id="active-run-row" style="display:none">
          <div class="run-badge" id="run-badge">#—</div>
          <div style="min-width:0">
            <div class="run-player" id="run-player">—</div>
            <div class="run-active">● Active</div>
          </div>
        </div>
      </div>
      <div class="card" id="party-card" style="display:none">
        <div class="card-title">Current Party <span id="party-run-lbl" style="color:#5090e0;font-weight:400"></span></div>
        <div class="party-grid" id="party-grid"></div>
      </div>
      <div id="no-run-msg" class="empty" style="display:none">No active run — start one in the Runs tab.</div>
    </div>
  </div>

  <!-- ── Runs ───────────────────────────────────────────────── -->
  <div id="tab-runs" class="tab-panel">
    <div id="runs-loading" class="loading">Loading…</div>
    <div id="runs-list"></div>
  </div>

  <!-- ── Links ─────────────────────────────────────────────── -->
  <div id="tab-links" class="tab-panel">
    <div class="card">
      <div class="card-title">Stats &amp; History</div>
      <div class="links-grid">
        <a class="btn btn-secondary" href="/history">Run History</a>
        <a class="btn btn-secondary" href="/shiny">Shinies</a>
        <a class="btn btn-secondary" href="/memorial">Memorial</a>
        <a class="btn btn-secondary" href="/timeline">Timeline</a>
        <a class="btn btn-secondary" href="/trainers">Trainers</a>
        <a class="btn btn-secondary" href="/species">Species</a>
        <a class="btn btn-secondary" href="/soullink">Soul Link</a>
        <a class="btn btn-secondary" href="/dashboard">Dashboard</a>
      </div>
    </div>
    <div class="card" style="margin-top:.5rem">
      <div class="card-title">Run Views (slot 0)</div>
      <div class="links-grid">
        <a class="btn btn-secondary" href="/0/party">Party</a>
        <a class="btn btn-secondary" href="/0/routes">Routes</a>
        <a class="btn btn-secondary" href="/0/caught">Caught</a>
        <a class="btn btn-secondary" href="/0/dead">Dead</a>
        <a class="btn btn-secondary" href="/0/types">Types</a>
        <a class="btn btn-secondary" href="/0/moves">Moves</a>
        <a class="btn btn-secondary" href="/0/items">Items</a>
        <a class="btn btn-secondary" href="/0/box">Box</a>
      </div>
    </div>
  </div>

  <!-- ── Settings ──────────────────────────────────────────── -->
  <div id="tab-settings" class="tab-panel">
    <div class="card">
      <div class="card-title">Account</div>
      <div class="setting-row">
        <div>
          <div class="setting-label" id="settings-user">—</div>
          <div class="setting-desc">Logged in as</div>
        </div>
      </div>
      <div class="setting-row">
        <div>
          <div class="setting-label">Log Out</div>
          <div class="setting-desc">Sign out of this device</div>
        </div>
        <button class="btn btn-danger btn-sm" onclick="doLogout()">Log Out</button>
      </div>
    </div>
    <div class="card">
      <div class="card-title">Display</div>
      <div class="setting-row">
        <div>
          <div class="setting-label">Desktop Mode</div>
          <div class="setting-desc">Switch to the full desktop interface</div>
        </div>
        <button class="btn btn-secondary btn-sm" onclick="enableDesktop()">Switch</button>
      </div>
    </div>
    <div class="card">
      <div class="card-title">Integrations</div>
      <div class="setting-row">
        <div>
          <div class="setting-label">Twitch / Discord / OBS</div>
          <div class="setting-desc">Manage per-user integration settings</div>
        </div>
        <a class="btn btn-secondary btn-sm" href="/integrations">Open</a>
      </div>
      <div class="setting-row">
        <div>
          <div class="setting-label">Guide / Help</div>
          <div class="setting-desc">API, OBS, bots, webhooks</div>
        </div>
        <a class="btn btn-secondary btn-sm" href="/guide">Open</a>
      </div>
    </div>
  </div>
</div>

<!-- Bottom navigation -->
<nav class="bottom-nav">
  <button class="nav-btn active" id="nav-home" onclick="showTab('home')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
    Home
  </button>
  <button class="nav-btn" id="nav-runs" onclick="showTab('runs')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></svg>
    Runs
  </button>
  <button class="nav-btn" id="nav-links" onclick="showTab('links')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>
    Links
  </button>
  <button class="nav-btn" id="nav-settings" onclick="showTab('settings')">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
    Settings
  </button>
</nav>

<script>
let SESSION=null;

function authHdr(){return {};}
function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function fmtDate(s){if(!s)return'—';try{return new Date(s).toLocaleDateString();}catch{return s;}}

function showTab(name){
  document.querySelectorAll('.tab-panel').forEach(p=>p.classList.remove('active'));
  document.querySelectorAll('.nav-btn').forEach(b=>b.classList.remove('active'));
  document.getElementById('tab-'+name).classList.add('active');
  document.getElementById('nav-'+name).classList.add('active');
}

function enableDesktop(){
  localStorage.setItem('desktop_mode','1');
  window.location.href='/dashboard';
}

async function doLogout(){
  await fetch('/api/logout',{method:'POST'}).catch(()=>null);
  window.location.href='/';
}

async function doEndRun(runId){
  if(!confirm('End run #'+runId+'? This cannot be undone.'))return;
  const r=await fetch('/api/run/'+runId+'/end',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(!r){alert('Network error.');return;}
  if(r.ok){loadHome();loadRuns();}
  else{const d=await r.json().catch(()=>({}));alert(d.error||'Could not end run.');}
}

async function loadHome(){
  document.getElementById('home-loading').style.display='';
  document.getElementById('home-content').style.display='none';
  const r=await fetch('/api/me/dashboard',{headers:authHdr()}).catch(()=>null);
  document.getElementById('home-loading').style.display='none';
  if(!r||!r.ok){document.getElementById('home-loading').textContent='Could not load data.';document.getElementById('home-loading').style.display='';return;}
  const d=await r.json();
  document.getElementById('home-content').style.display='';

  const s=d.stats||{};
  document.getElementById('stat-runs').textContent=s.total_runs??'—';
  document.getElementById('stat-catches').textContent=s.total_catches??'—';
  document.getElementById('stat-deaths').textContent=s.total_deaths??'—';

  const runs=d.open_runs||[];
  const activeRunRow=document.getElementById('active-run-row');
  if(runs.length){
    const run=runs[0];
    document.getElementById('run-badge').textContent='#'+run.id;
    document.getElementById('run-player').textContent=run.player_name||('Run #'+run.id);
    activeRunRow.style.display='flex';
  }else{
    activeRunRow.style.display='none';
  }

  const party=d.recent_party||[];
  const partyCard=document.getElementById('party-card');
  const noRunMsg=document.getElementById('no-run-msg');
  if(party.length){
    const grid=document.getElementById('party-grid');
    grid.innerHTML='';
    if(runs.length)document.getElementById('party-run-lbl').textContent='Run #'+runs[0].id;
    for(const mon of party){
      const el=document.createElement('div');
      el.className='party-mon';
      const hp=mon.hp_current!=null&&mon.hp_max?Math.round(mon.hp_current/mon.hp_max*100):null;
      const hpCol=hp==null?'#7dce7d':hp>50?'#7dce7d':hp>25?'#d0c060':'#e94560';
      el.innerHTML=
        '<div class="mon-name">'+esc(mon.nickname)+(mon.is_shiny?' <span class="shiny-star">★</span>':'')+'</div>'
        +'<div class="mon-species">'+esc(mon.species_name)+'</div>'
        +'<div class="mon-level">Lv '+mon.level+'</div>'
        +(hp!=null?'<div class="mon-hp-bar"><div class="mon-hp-fill" style="width:'+hp+'%;background:'+hpCol+'"></div></div>':'');
      grid.appendChild(el);
    }
    partyCard.style.display='';
    noRunMsg.style.display='none';
  }else{
    partyCard.style.display='none';
    noRunMsg.style.display=runs.length?'none':'';
  }
}

async function loadRuns(){
  const list=document.getElementById('runs-list');
  list.innerHTML='';
  document.getElementById('runs-loading').style.display='';
  const r=await fetch('/api/runs',{headers:authHdr()}).catch(()=>null);
  document.getElementById('runs-loading').style.display='none';
  if(!r||!r.ok){list.innerHTML='<div class="empty">Could not load runs.</div>';return;}
  const d=await r.json();
  const runs=d.runs||[];
  if(!runs.length){list.innerHTML='<div class="empty">No runs yet.</div>';return;}
  for(const run of runs){
    const active=run.ended_at==null;
    const el=document.createElement('div');
    el.className='run-row';
    el.innerHTML=
      '<div class="run-row-info">'
        +'<div class="run-row-id">#'+run.id+(active?' &nbsp;<span style="color:#7dce7d;font-size:.68rem">● Active</span>':'')+'</div>'
        +'<div class="run-row-player">'+esc(run.player_name||'—')+'</div>'
        +'<div class="run-row-meta">'+fmtDate(run.started_at)+' · '+(run.catches??0)+' caught · '+(run.deaths??0)+' deaths</div>'
      +'</div>'
      +'<div class="run-row-actions">'
        +'<a class="btn btn-secondary btn-sm" href="/overlay?run='+run.id+'">View</a>'
        +(run.is_owner&&active?'<button class="btn btn-danger btn-sm" onclick="doEndRun('+run.id+')">End</button>':'')
      +'</div>';
    list.appendChild(el);
  }
}

async function init(){
  const r=await fetch('/api/me').catch(()=>null);
  if(!r||!r.ok){window.location.href='/';return;}
  const me=await r.json();
  document.getElementById('header-sub').textContent=me.username;
  document.getElementById('settings-user').textContent=me.username;
  loadHome();
  loadRuns();
}

init();
</script>
</body>
</html>"##;

pub(crate) async fn serve_mobile_page(
    State(state): State<WebState>,
) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        apply_page(MOBILE_APP_HTML, state.testing),
    )
}

/// `POST /api/run/:id/end` — mark a run as ended.
///
/// Requires auth. The caller must own the run and the run must be active.
pub(crate) async fn api_end_run(
    State(_state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::end_run_by_id(run_id, user.id)
    }).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))),
        Ok(Err(e)) if e.contains("do not own") || e.contains("not found") => {
            (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": e })))
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "task panicked" }))),
    }
}

///
/// Requires auth. The caller must own the run.
/// Body: `{ "username": "..." }`
pub(crate) async fn api_run_invite(
    State(_state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
    axum::Json(body): axum::Json<InviteBody>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let username = body.username.trim().to_string();
    let result = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::invite_user_to_run(run_id, user.id, &username)
    }).await;
    match result {
        Ok(Ok(invite_id)) => (StatusCode::OK, axum::Json(serde_json::json!({ "invite_id": invite_id }))),
        Ok(Err(e)) if e.contains("do not own") || e.contains("not found") => {
            (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": e })))
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/run/:id/invites` — list all invites for a run (owner view).
pub(crate) async fn api_run_invites(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_run_invites_json(&conn, run_id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/accept` — accept an invite to a run.
pub(crate) async fn api_run_invite_accept(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    api_run_invite_respond(headers, run_id, true).await
}

/// `POST /api/run/:id/invite/decline` — decline an invite to a run.
pub(crate) async fn api_run_invite_decline(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    api_run_invite_respond(headers, run_id, false).await
}

pub(crate) async fn api_run_invite_respond(
    headers: axum::http::HeaderMap,
    run_id: u32,
    accept: bool,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::respond_to_invite(run_id, user.id, accept)
    }).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/request` — request access to a run.
///
/// Any authenticated user who does not own the run may call this.
pub(crate) async fn api_run_invite_request(
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::request_run_invite(run_id, user.id)
    }).await;
    match result {
        Ok(Ok(invite_id)) => (StatusCode::OK, axum::Json(serde_json::json!({ "invite_id": invite_id }))),
        Ok(Err(e)) if e.contains("already own") => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) if e.contains("not found") => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/run/:id/invite/requests` — list pending access requests (owner only).
pub(crate) async fn api_run_invite_requests(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(run_id): Path<u32>,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, (StatusCode, String)> {
        let user = fire_red_database::validate_session(&token)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;
        let owner_id = fire_red_database::get_run_owner_id(run_id);
        if owner_id != Some(user.id) {
            return Err((StatusCode::FORBIDDEN, "only the run owner can view access requests".to_string()));
        }
        Ok(fire_red_database::get_run_invite_requests_json(&conn, run_id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err((status, e))) => (status, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `POST /api/run/:id/invite/request/:uid/approve` — approve an access request.
pub(crate) async fn api_run_invite_request_approve(
    headers: axum::http::HeaderMap,
    Path((run_id, requester_id)): Path<(u32, u32)>,
) -> impl IntoResponse {
    api_run_invite_request_respond(headers, run_id, requester_id, true).await
}

/// `POST /api/run/:id/invite/request/:uid/deny` — deny an access request.
pub(crate) async fn api_run_invite_request_deny(
    headers: axum::http::HeaderMap,
    Path((run_id, requester_id)): Path<(u32, u32)>,
) -> impl IntoResponse {
    api_run_invite_request_respond(headers, run_id, requester_id, false).await
}

pub(crate) async fn api_run_invite_request_respond(
    headers: axum::http::HeaderMap,
    run_id: u32,
    requester_id: u32,
    approve: bool,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        fire_red_database::respond_to_invite_request(run_id, requester_id, user.id, approve)
    }).await;
    match result {
        Ok(Ok(())) => (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true }))),
        Ok(Err(e)) if e.contains("do not own") => (StatusCode::FORBIDDEN, axum::Json(serde_json::json!({ "error": e }))),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/me/run_statuses` — map of run_id → access status for the caller.
pub(crate) async fn api_me_run_statuses(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_my_run_statuses_json(&conn, user.id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}

/// `GET /api/me/run_requests` — all pending access requests on runs the caller owns.
pub(crate) async fn api_me_run_requests(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let Some(token) = extract_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": "not authenticated" })));
    };
    let Some(conn) = state.db_conn else {
        return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "No database configured" })));
    };
    let result = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        let user = fire_red_database::validate_session(&token)?
            .ok_or_else(|| "session expired or invalid".to_string())?;
        Ok(fire_red_database::get_my_run_requests_json(&conn, user.id))
    }).await;
    match result {
        Ok(Ok(v)) => (StatusCode::OK, axum::Json(v)),
        Ok(Err(e)) => (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({ "error": e }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({ "error": "Task panicked" }))),
    }
}
