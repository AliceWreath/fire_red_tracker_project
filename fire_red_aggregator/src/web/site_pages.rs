//! Login, create-account, and run select/join pages.

use super::*;

// ---------------------------------------------------------------------------
// Create-account page
// ---------------------------------------------------------------------------

pub(crate) const REGISTER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Create Account – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:1rem}
.card{background:#16213e;border:1px solid #0f3460;border-radius:10px;padding:2rem;width:100%;max-width:400px}
h1{font-size:1.3rem;color:#e94560;margin-bottom:.3rem}
.sub{color:#888;font-size:.85rem;margin-bottom:1.5rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input{width:100%;padding:.55rem .75rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.95rem;margin-bottom:1rem}
input:focus{outline:none;border-color:#e94560}
.btn{display:block;width:100%;padding:.6rem;border:none;border-radius:4px;font-size:1rem;cursor:pointer}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-primary:disabled{background:#555;cursor:default}
.msg{margin-top:.9rem;padding:.55rem;border-radius:4px;text-align:center;font-size:.875rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.footer{margin-top:1.2rem;text-align:center;font-size:.82rem;color:#666}
.footer a{color:#5090e0;text-decoration:none}
.footer a:hover{text-decoration:underline}
.req{font-size:.75rem;color:#666;margin-top:-.6rem;margin-bottom:.9rem}
</style>
</head>
<body>
<div class="card">
  <h1>Create Account</h1>
  <p class="sub">Fire Red Tracker</p>
  <form id="reg-form" onsubmit="doRegister(event)">
    <label for="uname">Username</label>
    <input id="uname" type="text" placeholder="pick a username" autocomplete="username" required maxlength="64">
    <label for="upass">Password</label>
    <input id="upass" type="password" placeholder="at least 8 characters" autocomplete="new-password" required minlength="8">
    <p class="req">Minimum 8 characters.</p>
    <label for="upass2">Confirm Password</label>
    <input id="upass2" type="password" placeholder="repeat password" autocomplete="new-password" required>
    <button class="btn btn-primary" id="reg-btn" type="submit">Create Account</button>
  </form>
  <div id="msg" class="msg"></div>
  <div class="footer">Already have an account? <a href="/join">Log in on the join page</a></div>
</div>
<script>
async function doRegister(e){
  e.preventDefault();
  const msg=document.getElementById('msg');
  const btn=document.getElementById('reg-btn');
  msg.className='msg';
  const u=document.getElementById('uname').value.trim();
  const p=document.getElementById('upass').value;
  const p2=document.getElementById('upass2').value;
  if(p!==p2){msg.className='msg err';msg.textContent='Passwords do not match.';return;}
  if(p.length<8){msg.className='msg err';msg.textContent='Password must be at least 8 characters.';return;}
  btn.disabled=true;
  try{
    const r=await fetch('/api/users',{
      method:'POST',
      headers:{'Content-Type':'application/json'},
      body:JSON.stringify({username:u,password:p})
    });
    const d=await r.json();
    if(r.ok){
      msg.className='msg ok';
      msg.textContent='Account created! Redirecting to the join page…';
      setTimeout(()=>window.location.href='/join',1200);
    }else{
      msg.className='msg err';
      msg.textContent=d.error||('Error '+r.status);
      btn.disabled=false;
    }
  }catch(err){
    msg.className='msg err';
    msg.textContent='Network error: '+err.message;
    btn.disabled=false;
  }
}
</script>
</body>
</html>"#;

pub(crate) async fn serve_register() -> Html<&'static str> {
    Html(REGISTER_HTML)
}

// ---------------------------------------------------------------------------
// Run select / join page
// ---------------------------------------------------------------------------

pub(crate) const JOIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Run Select – Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale}
body{font-family:'Segoe UI',system-ui,sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;padding:2rem 1rem}
.container{max-width:860px;margin:0 auto}
h1{font-size:1.5rem;color:#e94560;margin-bottom:1.5rem;display:flex;align-items:center;justify-content:space-between;text-wrap:balance}
h1 .user-pill{display:inline-flex;align-items:center;gap:.6rem;background:#1a3a1a;border:1px solid #2d5a2d;border-radius:20px;padding:.2rem .85rem;font-size:.82rem;color:#7dce7d}
.section{background:#16213e;box-shadow:0 0 0 1px rgba(255,255,255,0.08);border-radius:12px;padding:1.5rem;margin-bottom:1.5rem;transition-property:box-shadow;transition-duration:150ms;transition-timing-function:ease-out}
.section:hover{box-shadow:0 0 0 1px rgba(255,255,255,0.13)}
.section-title{font-size:.95rem;font-weight:700;color:#ccc;margin-bottom:1rem;padding-bottom:.5rem;border-bottom:1px solid #1e3a6e;display:flex;align-items:center;justify-content:space-between}
.btn{display:inline-block;padding:.45rem 1.1rem;border:none;border-radius:6px;font-size:.875rem;cursor:pointer;text-decoration:none;line-height:1.4;transition-property:transform,background;transition-duration:150ms;transition-timing-function:ease-out}
.btn:not(:disabled):active{transform:scale(0.96)}
.btn-primary{background:#e94560;color:#fff}
.btn-primary:hover{background:#c73652}
.btn-primary:disabled{background:#555;cursor:default}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499}
.btn-secondary:hover{background:#253d6a}
.btn-success{background:#1a5c2e;color:#7dce7d;border:1px solid #2d8a2d}
.btn-success:hover{background:#1e6a34}
.btn-danger{background:#5c1a1a;color:#ce7d7d;border:1px solid #8a2d2d}
.btn-danger:hover{background:#6a1e1e}
.btn-warn{background:#4a3a00;color:#e0c040;border:1px solid #7a6000}
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
.run-active{color:#60e060;font-size:.75rem;font-weight:700}
.deaths{color:#e06060;font-variant-numeric:tabular-nums}
.catches{color:#60d060;font-variant-numeric:tabular-nums}
.badge-owner{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a5c;color:#5090e0;border:1px solid #2d5499;vertical-align:middle;margin-left:.3rem}
.badge-invited{display:inline-block;font-size:.65rem;padding:.1rem .35rem;border-radius:3px;background:#1a3a1a;color:#7dce7d;border:1px solid #2d8a2d;vertical-align:middle;margin-left:.3rem}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input[type=text],input[type=password],input[type=number],select{width:100%;padding:.5rem .7rem;background:#0f3460;border:1px solid #444;border-radius:4px;color:#eee;font-size:.9rem;margin-bottom:.8rem;transition-property:border-color;transition-duration:150ms;transition-timing-function:ease-out}
input:focus,select:focus{outline:none;border-color:#e94560}
select option{background:#0f3460}
.msg{margin-top:.6rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.85rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.loading{color:#888;font-size:.85rem}
.td-actions{text-align:right;white-space:nowrap;gap:3px;display:flex;justify-content:flex-end;flex-wrap:wrap}
.form-row{display:flex;gap:.6rem;align-items:flex-end}
.form-row>*{flex:1;margin-bottom:0}
.form-row .btn{flex:0 0 auto;white-space:nowrap}
.radio-group{display:flex;flex-direction:column;gap:.5rem;margin-bottom:.8rem}
.radio-group label{display:flex;align-items:center;gap:.5rem;font-size:.875rem;color:#ccc;cursor:pointer;margin:0}
.radio-group input[type=radio]{width:auto;margin:0}
.req-row{display:flex;align-items:center;gap:.6rem;padding:.5rem 0;border-bottom:1px solid rgba(255,255,255,0.05);flex-wrap:wrap}
.req-row:last-child{border-bottom:none}
.req-info{flex:1;font-size:.85rem}
.req-user{color:#eee;font-weight:600}
.req-run{color:#5090e0;font-size:.8rem}
</style>
</head>
<body>
<div class="container">
<h1>
  <span>Run Select</span>
  <span id="user-pill" class="user-pill" style="display:none"></span>
</h1>

<!-- ── Your Runs ───────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">
    <span>Your Runs</span>
    <button class="btn btn-success btn-sm" onclick="createRun()">+ New Run</button>
  </div>
  <div id="my-runs-status" class="loading">Loading…</div>
  <table id="my-runs-table" style="display:none">
    <thead><tr><th>#</th><th>Started</th><th>Status</th><th>Caught</th><th>Deaths</th><th></th></tr></thead>
    <tbody id="my-runs-body"></tbody>
  </table>
  <div id="msg-my-run" class="msg"></div>
</div>

<!-- ── Pending Invites ─────────────────────────────────────────────── -->
<div class="section" id="pending-invites-section" style="display:none">
  <div class="section-title">Pending Invites</div>
  <div id="pending-invites-list"></div>
</div>

<!-- ── All Runs ────────────────────────────────────────────────────── -->
<div class="section">
  <div class="section-title">All Runs</div>
  <div id="runs-status" class="loading">Loading runs…</div>
  <table id="runs-table" style="display:none">
    <thead><tr><th>#</th><th>Player</th><th>Started</th><th>Status</th><th>Caught</th><th>Deaths</th><th></th></tr></thead>
    <tbody id="runs-body"></tbody>
  </table>
  <div id="msg-run" class="msg"></div>
</div>

<!-- ── Pending Requests on Your Runs ──────────────────────────────── -->
<div class="section" id="requests-section" style="display:none">
  <div class="section-title">Access Requests on Your Runs</div>
  <div id="requests-list"></div>
</div>

<!-- ── Connect to RetroArch (direct mode only) ────────────────────── -->
<div class="section" id="direct-section" style="display:DIRECT_SECTION_DISPLAY">
  <div class="section-title">Connect to RetroArch</div>
  <p style="color:#aaa;font-size:.875rem;line-height:1.5;margin-bottom:1rem">Enter the IP of the machine running RetroArch. Network Commands must be enabled in RetroArch settings.</p>
  <form id="connect-form" onsubmit="doConnect(event)">
    <div class="form-row">
      <div><label for="c-host">RetroArch IP</label><input id="c-host" type="text" placeholder="192.168.1.x" required></div>
      <div style="flex:0 0 110px"><label for="c-port">Port</label><input id="c-port" type="number" value="DEFAULT_PORT" min="1" max="65535" required></div>
    </div>
    <label>Run</label>
    <div class="radio-group">
      <label><input type="radio" name="run-choice" value="new" checked onchange="updateRunPicker()"> Start a new run</label>
      <label><input type="radio" name="run-choice" value="existing" onchange="updateRunPicker()"> Resume an existing run</label>
    </div>
    <div id="run-picker-wrap" style="display:none">
      <label for="run-picker">Select run to resume</label>
      <select id="run-picker"><option value="">— loading runs —</option></select>
    </div>
    <p style="font-size:.78rem;color:#8a9;margin:.5rem 0 .8rem;padding:.45rem .6rem;background:#0a2a15;border:1px solid #1a5a2a;border-radius:4px;display:none" id="mp-hint">
      &#9432; Multi-player: to share history and analytics, all players must connect to the <strong>same run</strong>.
      Invited players should select the <em>[invited]</em> run above.
    </p>
    <button class="btn btn-primary" id="connect-btn" type="submit" style="width:100%">Connect</button>
  </form>
  <div id="msg-connect" class="msg"></div>
  <div id="active-hosts" style="display:none;margin-top:1rem;font-size:.8rem;color:#888">
    <strong>Currently connected hosts:</strong>
    <ul id="host-list" style="margin-top:.4rem;padding-left:1.2rem;color:#aaa"></ul>
  </div>
</div>

</div><!-- /container -->
<script>
const TOKEN_KEY='frt_session';
const CLIENT_IP='__CLIENT_IP__';
const DIRECT_PORT=DEFAULT_PORT;
const DIRECT_ACTIVE=DIRECT_MODE_ACTIVE;
let SESSION=null;
let ME=null;
let ALL_RUNS=[];
let MY_STATUSES={};// run_id (string) → 'owner'|'accepted'|'pending_invite'|'pending_request'

function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function fmtDate(iso){if(!iso)return'—';try{return new Date(iso).toLocaleDateString();}catch{return iso;}}
function authHdr(){return {};}
function openRunPage(runId,sel){
  const p=sel.value;sel.value='';if(!p)return;
  const url=p==='stats'?'/run/'+runId+'/stats':'/'+p+'?run='+runId;
  fetch('/api/me/active_run',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({run_id:runId})}).catch(()=>{});
  window.open(url,'_blank');
}

async function init(){
  const r=await fetch('/api/me').catch(()=>null);
  if(!r||!r.ok){window.location.href='/';return;}
  ME=await r.json();
  document.getElementById('user-pill').textContent='● '+ME.username;
  document.getElementById('user-pill').style.display='';
  await Promise.all([loadStatuses(),loadAllRuns()]);
  loadMyRuns();
  loadPendingInvites();
  loadAccessRequests();
  loadHosts();
}

async function loadStatuses(){
  const r=await fetch('/api/me/run_statuses',{headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){const d=await r.json();MY_STATUSES=d.statuses||{};}
}

async function loadAllRuns(){
  const st=document.getElementById('runs-status');
  try{
    const r=await fetch('/api/runs');
    if(r.ok){const d=await r.json();ALL_RUNS=d.runs||[];renderAllRuns();}
    else{st.textContent='No database connected.';}
  }catch(e){st.textContent='Could not load runs.';}
  populateRunPicker();
}

function renderAllRuns(){
  const st=document.getElementById('runs-status');
  const tbl=document.getElementById('runs-table');
  const tbody=document.getElementById('runs-body');
  if(!ALL_RUNS.length){st.textContent='No runs yet.';st.style.display='';tbl.style.display='none';return;}
  st.style.display='none';tbl.style.display='';
  tbody.innerHTML='';
  for(const run of ALL_RUNS){
    const status=MY_STATUSES[String(run.id)];
    const hasAccess=(status==='owner'||status==='accepted');
    const active=run.ended_at==null;
    let actions='';
    if(hasAccess){
      actions+='<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Live View</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select> ';
      actions+='<button class="btn btn-success btn-xs" onclick="resumeRun('+run.id+')">Resume</button>';
      if(DIRECT_ACTIVE&&active)actions+=' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open live view">Quick Connect</button>';
      if(status==='owner'&&active)actions+=' <button class="btn btn-danger btn-xs" onclick="doEndRun('+run.id+')">End Run</button>';
    }else if(status==='pending_request'){
      actions='<span style="color:#888;font-size:.78rem">Request pending…</span>';
    }else if(status==='pending_invite'){
      actions='<span style="color:#e0c040;font-size:.78rem">Invite pending</span>';
    }else{
      actions='<button class="btn btn-warn btn-xs" onclick="requestAccess('+run.id+',this)">Request Access</button>';
    }
    const ownerBadge=status==='owner'?'<span class="badge-owner">owner</span>'
                    :status==='accepted'?'<span class="badge-invited">invited</span>':'';
    const tr=document.createElement('tr');
    tr.innerHTML=
      '<td><span class="run-id">#'+run.id+'</span>'+ownerBadge+'</td>'
      +'<td>'+esc(run.player_name||'—')+'</td>'
      +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
      +'<td>'+(active?'<span class="run-active">● Active</span>':'<span style="color:#555;font-size:.8rem">ended</span>')+'</td>'
      +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
      +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
      +'<td class="td-actions">'+actions+'</td>';
    tbody.appendChild(tr);
  }
}

async function loadMyRuns(){
  if(!ME)return;
  const st=document.getElementById('my-runs-status');
  st.textContent='Loading…';st.style.display='';
  document.getElementById('my-runs-table').style.display='none';
  try{
    const r=await fetch('/api/user/'+ME.id+'/runs',{headers:authHdr()});
    if(!r.ok){st.textContent='Could not load your runs.';return;}
    const d=await r.json();
    const runs=d.runs||[];
    const tbody=document.getElementById('my-runs-body');
    const tbl=document.getElementById('my-runs-table');
    if(!runs.length){st.textContent='No runs yet.';st.style.display='';tbl.style.display='none';return;}
    st.style.display='none';tbl.style.display='';
    tbody.innerHTML='';
    for(const run of runs){
      const active=run.ended_at==null;
      const badge=run.is_owner?'<span class="badge-owner">owner</span>':'<span class="badge-invited">invited</span>';
      let actions=
        '<select class="page-select" onchange="openRunPage('+run.id+',this)"><option value="">Open page…</option><option value="overlay">Live View</option><option value="history">History</option><option value="stats">Stats</option><option value="shiny">Shiny</option><option value="memorial">Memorial</option><option value="trainers">Trainers</option><option value="timeline">Timeline</option></select> '
        +'<button class="btn btn-success btn-xs" onclick="resumeRun('+run.id+')">Resume</button>'
        +(DIRECT_ACTIVE&&active?' <button class="btn btn-connect btn-xs" onclick="quickConnect('+run.id+')" title="Connect your RetroArch and open live view">Quick Connect</button>':'')
        +(run.is_owner&&active?' <button class="btn btn-danger btn-xs" onclick="doEndRun('+run.id+')">End Run</button>':'');
      const tr=document.createElement('tr');
      tr.innerHTML=
        '<td><span class="run-id">#'+run.id+'</span>'+badge+'</td>'
        +'<td style="color:#888;font-size:.8rem">'+fmtDate(run.started_at)+'</td>'
        +'<td>'+(active?'<span class="run-active">● Active</span>':'<span style="color:#555;font-size:.8rem">ended</span>')+'</td>'
        +'<td><span class="catches">'+(run.catches??0)+'</span></td>'
        +'<td><span class="deaths">'+(run.deaths??0)+'</span></td>'
        +'<td class="td-actions">'+actions+'</td>';
      tbody.appendChild(tr);
    }
  }catch(e){document.getElementById('my-runs-status').textContent='Could not load your runs.';}
}

async function loadPendingInvites(){
  const sec=document.getElementById('pending-invites-section');
  const list=document.getElementById('pending-invites-list');
  const pending=Object.entries(MY_STATUSES)
    .filter(([,v])=>v==='pending_invite')
    .map(([id])=>parseInt(id,10));
  if(!pending.length){sec.style.display='none';return;}
  // Look up run details from ALL_RUNS
  list.innerHTML='';
  for(const runId of pending){
    const run=ALL_RUNS.find(r=>r.id===runId);
    if(!run)continue;
    const row=document.createElement('div');
    row.className='req-row';
    row.id='inv-row-'+runId;
    row.innerHTML=
      '<div class="req-info"><span class="req-user">Run #'+runId+'</span>'
      +' <span class="req-run">'+esc(run.player_name||'—')+'</span></div>'
      +'<button class="btn btn-success btn-sm" onclick="respondInvite('+runId+',true)">Accept</button>'
      +'<button class="btn btn-danger btn-sm" onclick="respondInvite('+runId+',false)">Decline</button>';
    list.appendChild(row);
  }
  if(list.children.length)sec.style.display='';
}

async function respondInvite(runId,accept){
  const ep=accept?'accept':'decline';
  const r=await fetch('/api/run/'+runId+'/invite/'+ep,{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('inv-row-'+runId);
    if(row)row.remove();
    const list=document.getElementById('pending-invites-list');
    if(!list.children.length)document.getElementById('pending-invites-section').style.display='none';
    MY_STATUSES[String(runId)]=accept?'accepted':undefined;
    if(accept){await loadStatuses();loadMyRuns();renderAllRuns();}
    else{delete MY_STATUSES[String(runId)];renderAllRuns();}
  }
}

async function createRun(){
  const msg=document.getElementById('msg-my-run');
  const r=await fetch('/api/run',{method:'POST',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify({})}).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    msg.className='msg ok';msg.textContent='Created run #'+d.run_id+'.';
    setTimeout(async()=>{await loadStatuses();await loadAllRuns();loadMyRuns();},600);
  }else{
    msg.className='msg err';msg.textContent=d.error||'Failed.';
  }
}

async function resumeRun(runId){
  const r=await fetch('/api/run/'+runId+'/resume',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const msg=document.getElementById('msg-my-run');
    msg.className='msg ok';msg.textContent='Run #'+runId+' set as active.';
  }else if(r){
    const d=await r.json().catch(()=>({}));
    const msg=document.getElementById('msg-my-run');
    msg.className='msg err';msg.textContent=d.error||'Could not resume run.';
  }
}

async function doEndRun(runId){
  if(!confirm('End run #'+runId+'? This cannot be undone.'))return;
  const r=await fetch('/api/run/'+runId+'/end',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(!r){alert('Network error.');return;}
  if(r.ok){
    loadAllRuns();
    loadMyRuns();
  }else{
    const d=await r.json().catch(()=>({}));
    alert(d.error||'Could not end run.');
  }
}

async function requestAccess(runId,btn){
  btn.disabled=true;
  const r=await fetch('/api/run/'+runId+'/invite/request',{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    MY_STATUSES[String(runId)]='pending_request';
    renderAllRuns();
  }else{
    btn.disabled=false;
    if(r){const d=await r.json().catch(()=>({}));alert(d.error||'Request failed.');}
  }
}

async function loadAccessRequests(){
  const r=await fetch('/api/me/run_requests',{headers:authHdr()}).catch(()=>null);
  if(!r||!r.ok)return;
  const d=await r.json();
  const reqs=d.requests||[];
  if(!reqs.length)return;
  const sec=document.getElementById('requests-section');
  const list=document.getElementById('requests-list');
  list.innerHTML='';
  for(const req of reqs){
    const row=document.createElement('div');
    row.className='req-row';
    row.id='req-row-'+req.invite_id;
    row.innerHTML=
      '<div class="req-info">'
        +'<span class="req-user">'+esc(req.username)+'</span>'
        +' <span class="req-run">wants access to Run #'+req.run_id+' ('+esc(req.player_name)+')</span>'
        +'<div style="color:#666;font-size:.75rem">'+fmtDate(req.created_at)+'</div>'
      +'</div>'
      +'<button class="btn btn-success btn-sm" onclick="respondRequest('+req.run_id+','+req.user_id+','+req.invite_id+',true)">Approve</button>'
      +'<button class="btn btn-danger btn-sm" onclick="respondRequest('+req.run_id+','+req.user_id+','+req.invite_id+',false)">Deny</button>';
    list.appendChild(row);
  }
  sec.style.display='';
}

async function respondRequest(runId,userId,inviteId,approve){
  const ep=approve?'approve':'deny';
  const r=await fetch('/api/run/'+runId+'/invite/request/'+userId+'/'+ep,{method:'POST',headers:authHdr()}).catch(()=>null);
  if(r&&r.ok){
    const row=document.getElementById('req-row-'+inviteId);
    if(row)row.remove();
    const list=document.getElementById('requests-list');
    if(!list.children.length)document.getElementById('requests-section').style.display='none';
  }
}

// ── Quick connect ────────────────────────────────────────────────────
async function quickConnect(runId){
  const msg=document.getElementById('msg-my-run');
  msg.className='msg';
  const r=await fetch('/api/direct/connect',{
    method:'POST',
    headers:{'Content-Type':'application/json',...authHdr()},
    body:JSON.stringify({host:CLIENT_IP,port:DIRECT_PORT,run_id:runId}),
  }).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    window.open('/overlay?run='+runId,'_blank');
  }else{
    msg.className='msg err';msg.textContent=d.error||'Connection failed.';
  }
}

// ── Direct mode ──────────────────────────────────────────────────────
function populateRunPicker(){
  const sel=document.getElementById('run-picker');
  sel.innerHTML='<option value="">— new run —</option>';
  let firstInvitedActive=null;
  let hasOwnActive=false;
  for(const run of ALL_RUNS){
    const status=MY_STATUSES[String(run.id)];
    if(status!=='owner'&&status!=='accepted')continue;
    const isInvited=status==='accepted';
    const active=run.ended_at==null;
    if(active&&!isInvited)hasOwnActive=true;
    if(active&&isInvited&&!firstInvitedActive)firstInvitedActive=run;
    const opt=document.createElement('option');
    opt.value=run.id;
    opt.textContent=(isInvited?'[invited] ':'[owner] ')+'#'+run.id+' '+(run.player_name||'Unknown')+' ('+fmtDate(run.started_at)+(active?'':', ended')+')';
    sel.appendChild(opt);
  }
  // If the user has an accepted active invite but no own active run, default to
  // "existing run" mode with that invited run pre-selected so their catches go
  // to the correct shared run rather than a brand-new unlinked run.
  if(firstInvitedActive&&!hasOwnActive){
    const radio=document.querySelector('input[name="run-choice"][value="existing"]');
    if(radio){radio.checked=true;updateRunPicker();}
    sel.value=String(firstInvitedActive.id);
  }
  // Show the multi-player hint whenever any invited run exists.
  const hint=document.getElementById('mp-hint');
  if(hint)hint.style.display=firstInvitedActive?'':'none';
}

function updateRunPicker(){
  const choice=document.querySelector('input[name="run-choice"]:checked').value;
  document.getElementById('run-picker-wrap').style.display=(choice==='existing'?'':'none');
}

async function doConnect(e){
  e.preventDefault();
  const host=document.getElementById('c-host').value.trim();
  const port=parseInt(document.getElementById('c-port').value,10);
  const choice=document.querySelector('input[name="run-choice"]:checked').value;
  const runIdVal=document.getElementById('run-picker').value;
  const run_id=choice==='existing'&&runIdVal?parseInt(runIdVal,10):null;
  const msg=document.getElementById('msg-connect');
  const btn=document.getElementById('connect-btn');
  msg.className='msg';btn.disabled=true;
  try{
    const body={host,port};
    if(run_id!=null)body.run_id=run_id;
    const r=await fetch('/api/direct/connect',{method:'POST',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify(body)});
    const d=await r.json();
    msg.className='msg '+(r.ok?'ok':'err');
    msg.textContent=r.ok?(d.message||'Connection request sent.'):(d.error||'Connection failed.');
  }catch(err){
    msg.className='msg err';msg.textContent='Request failed: '+err.message;
  }
  btn.disabled=false;
}

async function loadHosts(){
  try{
    const r=await fetch('/api/direct/hosts');
    if(r.ok){
      const d=await r.json();
      const el=document.getElementById('active-hosts');
      const ul=document.getElementById('host-list');
      ul.innerHTML='';
      if(d.hosts&&d.hosts.length>0){
        d.hosts.forEach(h=>{
          const li=document.createElement('li');
          li.style.cssText='display:flex;align-items:center;gap:.5rem;margin-bottom:.25rem';
          const span=document.createElement('span');span.textContent=h;
          const btn=document.createElement('button');
          btn.textContent='Disconnect';
          btn.style.cssText='font-size:.7rem;padding:1px 6px;cursor:pointer;background:#c0392b;color:#fff;border:none;border-radius:3px';
          btn.onclick=async()=>{
            btn.disabled=true;
            const [host,port]=h.split(':');
            const res=await fetch('/api/direct/connect',{method:'DELETE',headers:{'Content-Type':'application/json',...authHdr()},body:JSON.stringify({host,port:port?parseInt(port,10):undefined})}).catch(()=>null);
            if(res&&res.ok){li.remove();if(!ul.children.length)el.style.display='none';}
            else{btn.disabled=false;}
          };
          li.appendChild(span);li.appendChild(btn);ul.appendChild(li);
        });
        el.style.display='';
      }else{el.style.display='none';}
    }
  }catch(e){}
}

init();
</script>
</body>
</html>"#;

pub(crate) async fn serve_join(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebState>,
) -> impl IntoResponse {
    let direct_visible = if state.connector.is_some() { "block" } else { "none" };
    let default_port = state.connector.as_ref().map(|c| c.default_port).unwrap_or(55355);
    let client_ip = addr.ip().to_string();
    let direct_active = if state.connector.is_some() { "true" } else { "false" };
    let html = JOIN_HTML
        .replace("DIRECT_SECTION_DISPLAY", direct_visible)
        .replace("DIRECT_MODE_ACTIVE", direct_active)
        .replace("DEFAULT_PORT", &default_port.to_string())
        .replace("192.168.1.x", &client_ip)
        .replace("__CLIENT_IP__", &client_ip);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

#[derive(serde::Deserialize)]
pub(crate) struct DirectConnectBody {
    host: String,
    port: Option<u16>,
    /// Existing run ID to resume. Omit (or pass `null`) to start a new run.
    run_id: Option<u32>,
}

pub(crate) async fn api_direct_connect(
    State(state): State<WebState>,
    Extension(caller): Extension<User>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<DirectConnectBody>,
) -> impl IntoResponse {
    let Some(connector) = &state.connector else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Direct mode is not active."})),
        );
    };

    let host = body.host.trim().to_string();
    if host.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "host must not be empty"})),
        );
    }

    // When resuming an existing run, require auth and access.
    // Returns the authenticated user_id so we can record the active run.
    let mut authed_user_id: Option<u32> = None;
    if let Some(run_id) = body.run_id {
        let Some(token) = extract_bearer(&headers) else {
            return (StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "authentication required to resume a run"})));
        };
        let check = tokio::task::spawn_blocking(move || -> Result<u32, (StatusCode, String)> {
            let user = fire_red_database::validate_session(&token)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
                .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid".to_string()))?;
            match fire_red_database::user_can_access_run(run_id, user.id) {
                Ok(true) => Ok(user.id),
                Ok(false) => Err((StatusCode::FORBIDDEN, "you do not have access to this run".into())),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
            }
        }).await;
        match check {
            Ok(Ok(uid)) => { authed_user_id = Some(uid); }
            Ok(Err((status, e))) => return (status, axum::Json(serde_json::json!({"error": e}))),
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(serde_json::json!({"error": "Task panicked"}))),
        }
    }

    let port = body.port.unwrap_or(connector.default_port);

    // When targeting a specific run, always disconnect the host first so it
    // can switch away from whatever run (if any) it was previously polling.
    if body.run_id.is_some() {
        connector.disconnect(&host, port);
    }

    let accepted = connector.connect(host.clone(), port, body.run_id, Some(caller.id));

    // Record user → run association so the overlay can auto-detect it.
    if let (Some(uid), Some(run_id)) = (authed_user_id, body.run_id) {
        state.user_active_run.lock().unwrap().insert(uid, run_id);
    }

    if accepted {
        tracing::info!("Direct mode: /join accepted {}:{} (run={:?})", host, port, body.run_id);
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "message": "Connection request received. Your slot will appear in a few seconds \
                            once the ROM is identified."
            })),
        )
    } else {
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "message": "Already connected.",
                "already": true
            })),
        )
    }
}

pub(crate) async fn api_direct_hosts(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
) -> impl IntoResponse {
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    // Collect the direct_host values for slots this user can access.
    let my_hosts: HashSet<String> = {
        let slots = state.live_slots.lock_or_recover();
        slots
            .iter()
            .filter(|s| {
                let run_id = s.db.as_ref().and_then(|db| db.get_run_id());
                run_id.is_none_or(|rid| accessible.contains(&rid))
            })
            .filter_map(|s| s.direct_host.clone())
            .collect()
    };
    let all_hosts = state.connector.as_ref().map(|c| c.active_hosts()).unwrap_or_default();
    let hosts: Vec<String> = all_hosts.into_iter().filter(|h| my_hosts.contains(h)).collect();
    axum::Json(serde_json::json!({"hosts": hosts}))
}

#[derive(serde::Deserialize)]
pub(crate) struct DirectDisconnectBody {
    host: String,
    port: Option<u16>,
}

pub(crate) async fn api_direct_disconnect(
    State(state): State<WebState>,
    Extension(user): Extension<User>,
    axum::Json(body): axum::Json<DirectDisconnectBody>,
) -> impl IntoResponse {
    let Some(connector) = &state.connector else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Direct mode is not active."})),
        );
    };
    let host = body.host.trim().to_string();
    let port = body.port.unwrap_or(connector.default_port);
    let host_key = format!("{}:{}", host, port);
    // Only allow disconnect if the host belongs to a slot the user can access.
    let uid = user.id;
    let accessible = tokio::task::spawn_blocking(move || {
        fire_red_database::get_accessible_run_ids(uid)
    })
    .await
    .unwrap_or(Ok(HashSet::new()))
    .unwrap_or_default();
    let owns_host = {
        let slots = state.live_slots.lock_or_recover();
        slots.iter().any(|s| {
            if s.direct_host.as_deref() != Some(&host_key) {
                return false;
            }
            let run_id = s.db.as_ref().and_then(|db| db.get_run_id());
            run_id.is_none_or(|rid| accessible.contains(&rid))
        })
    };
    if !owns_host {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "access denied"})),
        );
    }
    if connector.disconnect(&host, port) {
        tracing::info!("Direct mode: disconnected {}:{}", host, port);
        (StatusCode::OK, axum::Json(serde_json::json!({"ok": true})))
    } else {
        (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Host not connected."})),
        )
    }
}

// ---------------------------------------------------------------------------
// Login / landing page  (served at "/")
// ---------------------------------------------------------------------------

pub(crate) const LOGIN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Fire Red Tracker</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
html{-webkit-font-smoothing:antialiased;-moz-osx-font-smoothing:grayscale}
body{font-family:'Segoe UI',system-ui,sans-serif;background:#1a1a2e;color:#eee;min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;padding:2rem 1rem}
@keyframes cardIn{from{opacity:0;transform:translateY(12px);filter:blur(4px)}to{opacity:1;transform:translateY(0);filter:blur(0)}}
.card{background:#16213e;box-shadow:0 0 0 1px rgba(255,255,255,0.1);border-radius:12px;padding:2rem;width:100%;max-width:380px;animation:cardIn 350ms ease-out both}
h1{font-size:1.4rem;color:#e94560;margin-bottom:.3rem;text-align:center;text-wrap:balance}
.subtitle{font-size:.8rem;color:#556;text-align:center;margin-bottom:1.8rem;text-wrap:pretty}
label{display:block;font-size:.85rem;color:#ccc;margin-bottom:.3rem}
input{width:100%;padding:.55rem .75rem;background:#0f3460;border:1px solid #444;border-radius:5px;color:#eee;font-size:.9rem;margin-bottom:1rem;transition-property:border-color;transition-duration:150ms;transition-timing-function:ease-out}
input:focus{outline:none;border-color:#e94560}
.btn{display:block;width:100%;padding:.55rem;border:none;border-radius:7px;font-size:.9rem;cursor:pointer;text-align:center;text-decoration:none;line-height:1.4;transition-property:transform,background;transition-duration:150ms;transition-timing-function:ease-out}
.btn:active{transform:scale(0.96)}
.btn-primary{background:#e94560;color:#fff;margin-bottom:.7rem}
.btn-primary:hover{background:#c73652}
.btn-secondary{background:#1e3a6e;color:#aad;border:1px solid #2d5499;margin-bottom:.5rem}
.btn-secondary:hover{background:#253d6a}
.msg{margin-top:.5rem;padding:.5rem;border-radius:4px;text-align:center;font-size:.82rem;display:none}
.ok{background:#1a4a1a;border:1px solid #2d8a2d;color:#7dce7d;display:block}
.err{background:#4a1a1a;border:1px solid #8a2d2d;color:#ce7d7d;display:block}
.divider{border:none;border-top:1px solid #1e3a6e;margin:1.2rem 0}
.links{display:flex;flex-direction:column;gap:.5rem}
.user-info{text-align:center;margin-bottom:1rem;font-size:.9rem;color:#7dce7d}
.hint{font-size:.75rem;color:#556;text-align:center;margin-top:.4rem;text-wrap:pretty}
</style>
</head>
<body>
<div class="card">
  <h1>🔴 Fire Red Tracker</h1>
  <p class="subtitle">Nuzlocke run tracker</p>

  <!-- Logged-out state -->
  <div id="login-wrap">
    <label for="uname">Username</label>
    <input id="uname" type="text" placeholder="your username" autocomplete="username">
    <label for="upass">Password</label>
    <input id="upass" type="password" placeholder="••••••••" autocomplete="current-password" onkeydown="if(event.key==='Enter')doLogin()">
    <button class="btn btn-primary" onclick="doLogin()">Log In</button>
    <p class="hint">No account? <a href="/register" style="color:#5090e0">Register here</a></p>
    <div id="msg-login" class="msg"></div>
  </div>

  <!-- Logged-in state -->
  <div id="loggedin-wrap" style="display:none">
    <div class="user-info" id="user-info"></div>
    <div class="links">
      <a class="btn btn-primary" href="/overlay" id="overlay-link">Overlay</a>
      <a class="btn btn-secondary" href="/dashboard">Dashboard</a>
      <a class="btn btn-secondary" href="/join">Join / Run Select</a>
      <a class="btn btn-secondary" href="/history">Run History</a>
    </div>
    <hr class="divider">
    <button class="btn btn-secondary" onclick="doLogout()">Log Out</button>
  </div>
</div>

<script>
// Session token is kept in-memory only — the HttpOnly frt_token cookie
// handles persistent authentication so tokens never touch localStorage.
let SESSION=null;

function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
function mobileTarget(){
  const mobile=/Android|iPhone|iPad|iPod/i.test(navigator.userAgent)||window.innerWidth<768;
  return (!localStorage.getItem('desktop_mode')&&mobile)?'/mobile':'/dashboard';
}

async function init(){
  // Cookie is sent automatically on same-origin requests; no explicit header needed.
  const r=await fetch('/api/me').catch(()=>null);
  if(r&&r.ok){
    window.location.href=mobileTarget();
  }
}

function showLoggedIn(me){
  document.getElementById('login-wrap').style.display='none';
  document.getElementById('loggedin-wrap').style.display='';
  document.getElementById('user-info').textContent='Logged in as '+esc(me.username);
}

async function doLogin(){
  const u=document.getElementById('uname').value.trim();
  const p=document.getElementById('upass').value;
  const msg=document.getElementById('msg-login');
  msg.className='msg';
  if(!u||!p){msg.className='msg err';msg.textContent='Enter username and password.';return;}
  const r=await fetch('/api/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({username:u,password:p})}).catch(()=>null);
  if(!r){msg.className='msg err';msg.textContent='Network error.';return;}
  const d=await r.json();
  if(r.ok){
    SESSION=d.token;
    window.location.href=mobileTarget();
  }else{
    msg.className='msg err';msg.textContent=d.error||'Login failed.';
  }
}

async function doLogout(){
  await fetch('/api/logout',{method:'POST'}).catch(()=>null);
  SESSION=null;
  document.getElementById('loggedin-wrap').style.display='none';
  document.getElementById('login-wrap').style.display='';
  document.getElementById('upass').value='';
}

init();
</script>
</body>
</html>"#;

pub(crate) async fn serve_login_page() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        LOGIN_HTML,
    )
}
