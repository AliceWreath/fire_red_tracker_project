// Shared overlay-page runtime, served at /static/overlay.js (public route —
// contains no data or secrets, so it must load without a session).
//
// Every overlay/stat page loads this before its inline script. It owns the
// boilerplate that used to be copy-pasted into each page (and drifted):
// ?token= forwarding for OBS browser sources, the /ws connect + reconnect
// loop, Bearer-token fetch, and the ?run= auto-resolve redirect.
//
//   FRT.params            URLSearchParams of the page URL
//   FRT.token             ?token= value or null (OBS browser-source auth)
//   FRT.withToken(url)    append the page ?token= to a same-origin URL
//   FRT.authHeaders()     { Authorization: 'Bearer <token>' } from ?token= or
//                         the frt_session login token; {} if neither is set
//   FRT.authFetch(url, opts)
//                         fetch() with 'Authorization: Bearer' set from
//                         ?token= or the frt_session login token, if present
//   FRT.connectWS(show, onMessage, onClose)
//                         connect /ws with the given ?show= filter (null for
//                         unfiltered) + token; JSON-parse each message into
//                         onMessage; reconnect with 1s -> 30s exponential
//                         backoff. Returns a handle: { socket, send(obj) }.
//   FRT.ensureRunParam()  if the URL lacks ?run=, look up the logged-in
//                         user's active run and reload with ?run=<id>
(function () {
    'use strict';

    const params = new URLSearchParams(window.location.search);
    const token  = params.get('token');

    function sessionToken() {
        return token || localStorage.getItem('frt_session');
    }

    function withToken(url) {
        if (!token) return url;
        return url + (url.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(token);
    }

    function authHeaders() {
        const t = sessionToken();
        return t ? { 'Authorization': 'Bearer ' + t } : {};
    }

    function authFetch(url, opts) {
        opts = opts || {};
        opts.headers = Object.assign(authHeaders(), opts.headers || {});
        return fetch(url, opts);
    }

    function connectWS(show, onMessage, onClose) {
        const path  = withToken(show ? '/ws?show=' + encodeURIComponent(show) : '/ws');
        const proto = location.protocol === 'https:' ? 'wss://' : 'ws://';
        const handle = {
            socket: null,
            send(obj) {
                if (handle.socket && handle.socket.readyState === WebSocket.OPEN)
                    handle.socket.send(typeof obj === 'string' ? obj : JSON.stringify(obj));
            },
        };
        let reconnectDelay = 1000;
        (function connect() {
            const ws = new WebSocket(proto + location.host + path);
            handle.socket = ws;
            ws.onopen    = () => { reconnectDelay = 1000; };
            ws.onmessage = e  => onMessage(JSON.parse(e.data));
            ws.onclose   = () => {
                if (onClose) onClose();
                setTimeout(connect, reconnectDelay);
                reconnectDelay = Math.min(reconnectDelay * 2, 30000);
            };
        })();
        return handle;
    }

    function ensureRunParam() {
        if (params.has('run')) return;
        const t = sessionToken();
        if (!t) return;
        fetch('/api/me/active_run', { headers: { 'Authorization': 'Bearer ' + t } })
            .then(r => r.ok ? r.json() : null)
            .then(d => {
                if (d && d.run_id) {
                    const sp = new URLSearchParams(window.location.search);
                    sp.set('run', d.run_id);
                    window.location.replace(window.location.pathname + '?' + sp.toString());
                }
            })
            .catch(() => {});
    }

    window.FRT = { params, token, withToken, authHeaders, authFetch, connectWS, ensureRunParam };
})();
