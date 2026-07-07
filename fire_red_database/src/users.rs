//! User accounts, sessions, per-user integrations, dashboard JSON, and
//! run invites / access requests.

use super::*;

// ---------------------------------------------------------------------------
// Public API — user accounts and sessions
// ---------------------------------------------------------------------------

const SESSION_TTL_SECS: u64 = 86_400 * 30; // 30 days

/// Register a new user. Returns `Err` if the username is already taken or the
/// password cannot be hashed.
///
/// The password is hashed with bcrypt (cost 12) before storage; the plaintext
/// is never written to the database.
pub fn create_user(username: &str, password: &str) -> Result<User, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("username must not be empty".to_string());
    }
    if password.len() < 8 {
        return Err("password must be at least 8 characters".to_string());
    }

    let hash = bcrypt::hash(password, 12)
        .map_err(|e| format!("Failed to hash password: {e}"))?;

    let Some(db) = db() else {
        return Err("database not initialised".to_string());
    };
    let mut state = db.lock_or_recover();

    let row = state.client.query_opt(
        "INSERT INTO users (username, password_hash, created_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (username) DO NOTHING
         RETURNING id, created_at",
        &[&username, &hash, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    let row = row.ok_or_else(|| format!("username '{}' is already taken", username))?;
    Ok(User {
        id: row.get::<_, i32>(0) as u32,
        username: username.to_string(),
        created_at: row.get::<_, i64>(1) as u64,
    })
}

/// Look up a user by username and verify their password.
///
/// Returns `Ok(Some(user))` on success, `Ok(None)` when credentials are wrong
/// or the user doesn't exist.
pub fn authenticate_user(username: &str, password: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();

    let row = match state.client.query_opt(
        "SELECT id, username, password_hash, created_at FROM users WHERE username = $1",
        &[&username],
    ) {
        Ok(r) => r,
        Err(e) => return Err(format!("DB error: {e}")),
    };

    let Some(row) = row else {
        tracing::warn!(username = %username, reason = "no such user", "login failed");
        return Ok(None);
    };
    let hash: String = row.get(2);

    let ok = bcrypt::verify(password, &hash)
        .map_err(|e| format!("bcrypt error: {e}"))?;

    if ok {
        tracing::info!(username = %username, "login succeeded");
        Ok(Some(User {
            id: row.get::<_, i32>(0) as u32,
            username: row.get(1),
            created_at: row.get::<_, i64>(3) as u64,
        }))
    } else {
        tracing::warn!(username = %username, reason = "wrong password", "login failed");
        Ok(None)
    }
}

/// Fetch a user by ID. Returns `Ok(None)` if the ID doesn't exist.
pub fn get_user_by_id(id: u32) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT id, username, created_at FROM users WHERE id = $1",
        &[&(id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }))
}

/// Return all registered users, ordered by ID.
pub fn list_users() -> Result<Vec<User>, String> {
    let Some(db) = db() else { return Ok(vec![]) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id, username, created_at FROM users ORDER BY id",
        &[],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.iter().map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }).collect())
}

/// Create a session token for a user. Returns the opaque bearer token.
///
/// Tokens expire after 30 days. Old expired sessions are pruned on each call.
pub fn create_session(user_id: u32) -> Result<String, String> {
    create_session_with_client_info(user_id, None, None)
}

/// Like [`create_session`], but records the client IP and User-Agent on the
/// session row so the dashboard session manager can show recognizable entries.
pub fn create_session_with_client_info(
    user_id: u32,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<String, String> {
    let Some(db) = db() else {
        return Err("database not initialised".to_string());
    };
    let mut state = db.lock_or_recover();

    // Prune expired sessions to keep the table tidy.
    let _ = state.client.execute(
        "DELETE FROM sessions WHERE expires_at < $1",
        &[&(unix_now() as i64)],
    );

    // Generate a token: SHA-256 of (user_id || 32 CSPRNG bytes).
    let mut rng_bytes = [0u8; 32];
    {
        use rand::RngCore;
        rand::rng().fill_bytes(&mut rng_bytes);
    }
    let mut hasher = Sha256::new();
    hasher.update(user_id.to_le_bytes());
    hasher.update(rng_bytes);
    let digest = hasher.finalize();
    let token: String = digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
        s
    });

    let now = unix_now() as i64;
    let expires = now + SESSION_TTL_SECS as i64;
    state.client.execute(
        "INSERT INTO sessions (token, user_id, created_at, expires_at, ip, user_agent)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[&token, &(user_id as i32), &now, &expires, &ip, &user_agent],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(token)
}

/// One row of [`list_sessions_for_user`]. Only a token prefix is exposed —
/// enough to identify a session for revocation, never enough to hijack it.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// First 12 hex chars of the 64-char token; the revocation handle.
    pub token_prefix: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    /// True when this row is the session making the request.
    pub current: bool,
}

/// Number of leading token characters exposed as the session handle.
/// 12 hex chars identify a session uniquely while leaving 52 hidden chars
/// (208 bits) so the handle is useless for authentication.
const SESSION_PREFIX_LEN: usize = 12;

/// Lists the active (non-expired) sessions belonging to `user_id`, newest
/// first. `current_token` marks which returned row is the caller's own.
pub fn list_sessions_for_user(
    user_id: u32,
    current_token: &str,
) -> Result<Vec<SessionInfo>, String> {
    let Some(db) = db() else { return Ok(Vec::new()) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT token, created_at, expires_at, ip, user_agent
         FROM sessions
         WHERE user_id = $1 AND expires_at > $2
         ORDER BY created_at DESC",
        &[&(user_id as i32), &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.iter().map(|r| {
        let token: String = r.get(0);
        SessionInfo {
            token_prefix: token.chars().take(SESSION_PREFIX_LEN).collect(),
            created_at: r.get::<_, i64>(1) as u64,
            expires_at: r.get::<_, i64>(2) as u64,
            ip: r.get(3),
            user_agent: r.get(4),
            current: token == current_token,
        }
    }).collect())
}

/// Revokes one of `user_id`'s own sessions by its 12-char token prefix.
///
/// The prefix must be exactly [`SESSION_PREFIX_LEN`] lowercase hex chars
/// (as returned by [`list_sessions_for_user`]); anything else is rejected
/// before touching the database. Returns `true` if a session was deleted.
pub fn delete_session_by_prefix(user_id: u32, prefix: &str) -> Result<bool, String> {
    if prefix.len() != SESSION_PREFIX_LEN
        || !prefix.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("invalid session prefix".to_string());
    }
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    // The prefix is validated hex, so it cannot contain LIKE wildcards.
    let n = state.client.execute(
        "DELETE FROM sessions WHERE user_id = $1 AND token LIKE $2 || '%'",
        &[&(user_id as i32), &prefix],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(n > 0)
}

/// Revokes every session belonging to `user_id` except `current_token`
/// ("sign out everywhere else"). Returns the number of sessions revoked.
pub fn delete_other_sessions(user_id: u32, current_token: &str) -> Result<u64, String> {
    let Some(db) = db() else { return Ok(0) };
    let mut state = db.lock_or_recover();
    state.client.execute(
        "DELETE FROM sessions WHERE user_id = $1 AND token <> $2",
        &[&(user_id as i32), &current_token],
    ).map_err(|e| format!("DB error: {e}"))
}

/// Validate a session token. Returns the associated `User` if the token is
/// valid and not expired; `Ok(None)` otherwise.
pub fn validate_session(token: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT u.id, u.username, u.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token = $1 AND s.expires_at > $2",
        &[&token, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.map(|r| User {
        id: r.get::<_, i32>(0) as u32,
        username: r.get(1),
        created_at: r.get::<_, i64>(2) as u64,
    }))
}

/// Revoke a session token (logout).
pub fn delete_session(token: &str) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute("DELETE FROM sessions WHERE token = $1", &[&token])
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Return all run IDs accessible to `user_id` — runs they own plus runs they
/// have an accepted invite for.  Used to filter live slot data per-user.
pub fn get_accessible_run_ids(user_id: u32) -> Result<std::collections::HashSet<u32>, String> {
    let Some(db) = db() else { return Ok(std::collections::HashSet::new()) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id FROM runs WHERE user_id = $1
         UNION
         SELECT run_id FROM run_invites WHERE invited_user = $1 AND status = 'accepted'",
        &[&(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.iter().map(|r| r.get::<_, i32>(0) as u32).collect())
}

// ---------------------------------------------------------------------------
// Per-user integration configs
// ---------------------------------------------------------------------------

/// Return the stored JSON config for `(user_id, kind)`, or `None` if not set.
pub fn get_user_integration(conn_str: &str, user_id: u32, kind: &str) -> Option<String> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return None };
    let row = client
        .query_opt(
            "SELECT config FROM user_integrations WHERE user_id = $1 AND kind = $2",
            &[&(user_id as i32), &kind],
        )
        .ok()??;
    Some(row.get(0))
}

/// Upsert a JSON config for `(user_id, kind)`. `config` is a JSON string.
pub fn set_user_integration(conn_str: &str, user_id: u32, kind: &str, config: &str) -> Result<(), String> {
    let conn_str = normalize_conn_str(conn_str);
    let mut client = Client::connect(&conn_str, NoTls)
        .map_err(|e| format!("DB connection failed: {e}"))?;
    let now = unix_now() as i64;
    client
        .execute(
            "INSERT INTO user_integrations (user_id, kind, config, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, kind) DO UPDATE
             SET config = EXCLUDED.config, updated_at = EXCLUDED.updated_at",
            &[&(user_id as i32), &kind, &config, &now],
        )
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Delete the integration config for `(user_id, kind)`. Returns `true` if a row was deleted.
pub fn delete_user_integration(conn_str: &str, user_id: u32, kind: &str) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "DELETE FROM user_integrations WHERE user_id = $1 AND kind = $2",
            &[&(user_id as i32), &kind],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Return all integration configs for a user as `{ kind: config_json_string }`.
pub fn list_user_integrations(conn_str: &str, user_id: u32) -> serde_json::Value {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else {
        return serde_json::json!({});
    };
    let rows = match client.query(
        "SELECT kind, config FROM user_integrations WHERE user_id = $1 ORDER BY kind",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(_) => return serde_json::json!({}),
    };
    let mut map = serde_json::Map::new();
    for row in &rows {
        let kind: String = row.get(0);
        let config_str: String = row.get(1);
        let val: serde_json::Value =
            serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Null);
        map.insert(kind, val);
    }
    serde_json::Value::Object(map)
}

/// Associate an existing run with a user account.
pub fn link_run_to_user(run_id: u32, user_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute(
        "UPDATE runs SET user_id = $1 WHERE id = $2 AND user_id IS NULL",
        &[&(user_id as i32), &(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Create a new run for a direct-mode slot **without** touching the global
/// `DbState.run_id`.  The caller must then call [`set_thread_run_id`] on the
/// game-loop thread so DB writes from that thread go to this run.
pub fn create_run_for_slot(player_name: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Err("No database configured".into()) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_one(
        "INSERT INTO runs (player_name, started_at) VALUES ($1, $2) RETURNING id",
        &[&player_name, &(unix_now() as i64)],
    ).map_err(|e| format!("Failed to create run: {e}"))?;
    Ok(row.get::<_, i32>(0) as u32)
}

/// Delete a run row created by this process (used to clean up orphan rows when
/// direct-mode setup fails after the run was already inserted).
pub fn delete_run(run_id: u32) -> Result<(), String> {
    let Some(db) = db() else { return Ok(()) };
    let mut state = db.lock_or_recover();
    state.client.execute(
        "DELETE FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}

/// Verify that a run with the given ID exists (for resuming in direct mode).
///
/// Returns `Ok(true)` if found, `Ok(false)` if not, `Err` on DB error.
/// Does **not** set the global active run — the caller should call
/// [`set_thread_run_id`] on the game-loop thread afterward.
pub fn run_exists(run_id: u32) -> Result<bool, String> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT 1 FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(!rows.is_empty())
}

/// Look up a user by username. Returns `None` if no account with that name exists.
pub fn get_user_by_username(username: &str) -> Result<Option<User>, String> {
    let Some(db) = db() else { return Ok(None) };
    let mut state = db.lock_or_recover();
    let rows = state.client.query(
        "SELECT id, username, created_at FROM users WHERE username = $1",
        &[&username],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(rows.first().map(|row| User {
        id: row.get::<_, i32>(0) as u32,
        username: row.get(1),
        created_at: row.get::<_, i64>(2) as u64,
    }))
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

/// Returns a dashboard JSON blob for the given user:
/// - `user`: `{ id, username }`
/// - `open_runs`: runs owned or accepted-invite runs that have no `ended_at`
/// - `stats`: totals across all accessible runs `{ deaths, catches, encounters, runs }`
/// - `recent_party`: alive caught_pokemon from the most recent open run (up to 6)
/// - `pending_invites`: invites waiting for this user's response
pub fn user_dashboard_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };

    // User info
    let user_row = match client.query_opt(
        "SELECT id, username FROM users WHERE id = $1",
        &[&(user_id as i32)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return serde_json::json!({ "error": "user not found" }),
        Err(e) => return serde_json::json!({ "error": format!("DB error: {e}") }),
    };
    let username: String = user_row.get(1);

    // Open runs (owned or accepted invite)
    let open_rows = match client.query(
        "SELECT r.id, r.player_name, r.started_at,
                COUNT(DISTINCT d.personality) AS deaths,
                COUNT(DISTINCT c.personality) AS catches,
                (r.user_id = $1)              AS is_owner
         FROM runs r
         LEFT JOIN dead_pokemon   d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         WHERE r.ended_at IS NULL
           AND (
               r.user_id = $1
               OR EXISTS (
                   SELECT 1 FROM run_invites ri
                   WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
               )
           )
         GROUP BY r.id, r.user_id
         ORDER BY (r.user_id = $1) DESC, r.id DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let open_runs: Vec<serde_json::Value> = open_rows.iter().map(|row| {
        let started: i64 = row.get(2);
        serde_json::json!({
            "id":          row.get::<_, i32>(0),
            "player_name": row.get::<_, String>(1),
            "started_at":  format_timestamp(started as u64),
            "deaths":      row.get::<_, i64>(3),
            "catches":     row.get::<_, i64>(4),
            "is_owner":    row.get::<_, bool>(5),
        })
    }).collect();

    // Aggregate stats across all accessible runs (open and closed)
    let stats_row = match client.query_opt(
        "SELECT COUNT(DISTINCT r.id)           AS run_count,
                COUNT(DISTINCT d.personality)  AS deaths,
                COUNT(DISTINCT c.personality)  AS catches,
                COUNT(DISTINCT e.id)           AS encounters
         FROM runs r
         LEFT JOIN dead_pokemon   d ON d.run_id = r.id
         LEFT JOIN caught_pokemon c ON c.run_id = r.id
         LEFT JOIN encounters     e ON e.run_id = r.id
         WHERE r.user_id = $1
            OR EXISTS (
                SELECT 1 FROM run_invites ri
                WHERE ri.run_id = r.id AND ri.invited_user = $1 AND ri.status = 'accepted'
            )",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Stats query failed: {e}") }),
    };
    let stats = stats_row.map(|row| serde_json::json!({
        "runs":       row.get::<_, i64>(0),
        "deaths":     row.get::<_, i64>(1),
        "catches":    row.get::<_, i64>(2),
        "encounters": row.get::<_, i64>(3),
    })).unwrap_or_else(|| serde_json::json!({ "runs": 0, "deaths": 0, "catches": 0, "encounters": 0 }));

    // Most recent party: alive caught pokemon from most recent open run
    let recent_party: Vec<serde_json::Value> = if let Some(first) = open_rows.first() {
        let run_id: i32 = first.get(0);
        match client.query(
            "SELECT cp.nickname, cp.species_name, cp.level, cp.is_shiny
             FROM caught_pokemon cp
             WHERE cp.run_id = $1
               AND NOT EXISTS (
                   SELECT 1 FROM dead_pokemon dp
                   WHERE dp.run_id = cp.run_id AND dp.personality = cp.personality
               )
             ORDER BY cp.caught_at ASC
             LIMIT 6",
            &[&run_id],
        ) {
            Ok(rows) => rows.iter().map(|row| serde_json::json!({
                "nickname":     row.get::<_, String>(0),
                "species_name": row.get::<_, String>(1),
                "level":        row.get::<_, i32>(2),
                "is_shiny":     row.get::<_, bool>(3),
            })).collect(),
            Err(_) => vec![],
        }
    } else {
        vec![]
    };

    // Pending invites for this user
    let invite_rows = match client.query(
        "SELECT ri.id, r.id AS run_id, r.player_name, u.username AS invited_by, ri.created_at
         FROM run_invites ri
         JOIN runs  r ON r.id  = ri.run_id
         JOIN users u ON u.id  = ri.invited_by
         WHERE ri.invited_user = $1 AND ri.status = 'pending' AND ri.is_request = FALSE
         ORDER BY ri.created_at DESC",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Invite query failed: {e}") }),
    };
    let pending_invites: Vec<serde_json::Value> = invite_rows.iter().map(|row| {
        let created: i64 = row.get(4);
        serde_json::json!({
            "invite_id":   row.get::<_, i32>(0),
            "run_id":      row.get::<_, i32>(1),
            "player_name": row.get::<_, String>(2),
            "invited_by":  row.get::<_, String>(3),
            "created_at":  format_timestamp(created as u64),
        })
    }).collect();

    serde_json::json!({
        "user": { "id": user_id, "username": username },
        "open_runs": open_runs,
        "stats": stats,
        "recent_party": recent_party,
        "pending_invites": pending_invites,
    })
}

// ---------------------------------------------------------------------------
// Run invites
// ---------------------------------------------------------------------------

/// Invite another user (by username) to collaborate on a run.
///
/// Only the run owner (`runs.user_id`) may invite others.
/// Returns `Ok(invite_id)` on success or `Err(message)` on failure.
pub fn invite_user_to_run(run_id: u32, inviter_user_id: u32, invitee_username: &str) -> Result<u32, String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    // Verify caller owns the run.
    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner_id: Option<i32> = row.as_ref().and_then(|r| r.get(0));
    if owner_id != Some(inviter_user_id as i32) {
        return Err("you do not own this run".to_string());
    }

    // Resolve invitee.
    let inv_row = state.client.query_opt(
        "SELECT id FROM users WHERE username = $1",
        &[&invitee_username],
    ).map_err(|e| format!("DB error: {e}"))?;
    let invitee_id: i32 = inv_row
        .ok_or_else(|| format!("user '{}' not found", invitee_username))?
        .get(0);

    if invitee_id == inviter_user_id as i32 {
        return Err("cannot invite yourself".to_string());
    }

    let result = state.client.query_one(
        "INSERT INTO run_invites (run_id, invited_by, invited_user, is_request, created_at)
         VALUES ($1, $2, $3, FALSE, $4)
         ON CONFLICT (run_id, invited_user) DO UPDATE
             SET status = 'pending', is_request = FALSE, responded_at = NULL, created_at = EXCLUDED.created_at
         RETURNING id",
        &[&(run_id as i32), &(inviter_user_id as i32), &invitee_id, &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(result.get::<_, i32>(0) as u32)
}

/// Accept or decline a run invite.
///
/// The responding user must be `invited_user` for the invite.
/// Returns `Ok(())` or `Err(reason)`.
pub fn respond_to_invite(run_id: u32, user_id: u32, accept: bool) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();
    let status = if accept { "accepted" } else { "declined" };
    let rows_affected = state.client.execute(
        "UPDATE run_invites SET status = $1, responded_at = $2
         WHERE run_id = $3 AND invited_user = $4 AND is_request = FALSE AND status = 'pending'",
        &[&status, &(unix_now() as i64), &(run_id as i32), &(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    if rows_affected == 0 {
        Err("no pending invite found for this run".to_string())
    } else {
        Ok(())
    }
}

/// List all invites for a run (owner view).
///
/// Returns `{ "invites": [...] }` or `{ "error": "..." }`.
pub fn get_run_invites_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, u.username AS invitee, ub.username AS invited_by, ri.status, ri.created_at, ri.responded_at
         FROM run_invites ri
         JOIN users u  ON u.id  = ri.invited_user
         JOIN users ub ON ub.id = ri.invited_by
         WHERE ri.run_id = $1 AND ri.is_request = FALSE
         ORDER BY ri.created_at DESC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let invites: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(4);
        let responded: Option<i64> = row.get(5);
        serde_json::json!({
            "invite_id":    row.get::<_, i32>(0),
            "invitee":      row.get::<_, String>(1),
            "invited_by":   row.get::<_, String>(2),
            "status":       row.get::<_, String>(3),
            "created_at":   format_timestamp(created as u64),
            "responded_at": responded.map(|t| format_timestamp(t as u64)),
        })
    }).collect();
    serde_json::json!({ "run_id": run_id, "invites": invites })
}

/// Return `true` if `user_id` owns or has an accepted invite for `run_id`.
pub fn user_can_access_run(run_id: u32, user_id: u32) -> Result<bool, String> {
    let Some(db) = db() else { return Ok(false) };
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT 1 FROM runs WHERE id = $1 AND user_id = $2
         UNION ALL
         SELECT 1 FROM run_invites
         WHERE run_id = $1 AND invited_user = $2 AND status = 'accepted'
         LIMIT 1",
        &[&(run_id as i32), &(user_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    Ok(row.is_some())
}

/// Return the user_id that owns `run_id`, or `None` if not found or unowned.
pub fn get_run_owner_id(run_id: u32) -> Option<u32> {
    let db = db()?;
    let mut state = db.lock_or_recover();
    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).ok()??;
    let owner: Option<i32> = row.get(0);
    owner.map(|v| v as u32)
}

/// Submit an access request for a run the caller does not own.
///
/// Reuses the `run_invites` table with `is_request = TRUE` and
/// `invited_by = invited_user = requester_id`.  If a prior request (or
/// invite) exists for this `(run_id, user)` pair it is reset to pending.
/// Returns the invite-row id on success.
pub fn request_run_invite(run_id: u32, requester_id: u32) -> Result<u32, String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    // Run must exist.
    let run_row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner: Option<i32> = run_row
        .as_ref()
        .ok_or_else(|| format!("run #{run_id} not found"))?
        .get(0);

    if owner.is_none() {
        return Err("this run has no owner and cannot accept access requests".to_string());
    }

    if owner == Some(requester_id as i32) {
        return Err("you already own this run".to_string());
    }

    let row = state.client.query_one(
        "INSERT INTO run_invites (run_id, invited_by, invited_user, is_request, created_at)
         VALUES ($1, $2, $2, TRUE, $3)
         ON CONFLICT (run_id, invited_user) DO UPDATE
             SET status = 'pending', is_request = TRUE,
                 responded_at = NULL, created_at = EXCLUDED.created_at
         RETURNING id",
        &[&(run_id as i32), &(requester_id as i32), &(unix_now() as i64)],
    ).map_err(|e| format!("DB error: {e}"))?;

    Ok(row.get::<_, i32>(0) as u32)
}

/// Approve or deny an access request.  `approver_id` must own the run.
pub fn respond_to_invite_request(run_id: u32, requester_id: u32, approver_id: u32, approve: bool) -> Result<(), String> {
    let Some(db) = db() else { return Err("database not initialised".to_string()) };
    let mut state = db.lock_or_recover();

    let row = state.client.query_opt(
        "SELECT user_id FROM runs WHERE id = $1",
        &[&(run_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;
    let owner: Option<i32> = row.as_ref().and_then(|r| r.get(0));
    if owner != Some(approver_id as i32) {
        return Err("you do not own this run".to_string());
    }

    let status = if approve { "accepted" } else { "declined" };
    let n = state.client.execute(
        "UPDATE run_invites SET status = $1, responded_at = $2
         WHERE run_id = $3 AND invited_user = $4 AND is_request = TRUE AND status = 'pending'",
        &[&status, &(unix_now() as i64), &(run_id as i32), &(requester_id as i32)],
    ).map_err(|e| format!("DB error: {e}"))?;

    if n == 0 { Err("no pending request found".to_string()) } else { Ok(()) }
}

/// List pending access requests for a run (owner view).
pub fn get_run_invite_requests_json(conn_str: &str, run_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, u.id AS uid, u.username, ri.created_at
         FROM run_invites ri
         JOIN users u ON u.id = ri.invited_user
         WHERE ri.run_id = $1 AND ri.is_request = TRUE AND ri.status = 'pending'
         ORDER BY ri.created_at ASC",
        &[&(run_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let requests: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(3);
        serde_json::json!({
            "invite_id":  row.get::<_, i32>(0),
            "user_id":    row.get::<_, i32>(1),
            "username":   row.get::<_, String>(2),
            "created_at": format_timestamp(created as u64),
        })
    }).collect();
    serde_json::json!({ "run_id": run_id, "requests": requests })
}

/// Return each run's access status for the given user.
///
/// `"owner"` | `"accepted"` | `"pending_invite"` | `"pending_request"`.
/// Declined / non-existent entries are omitted.
pub fn get_my_run_statuses_json(conn_str: &str, user_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let owned = match client.query(
        "SELECT id FROM runs WHERE user_id = $1",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let invite_rows = match client.query(
        "SELECT run_id, status, is_request FROM run_invites WHERE invited_user = $1",
        &[&(user_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Invite query failed: {e}") }),
    };

    let mut statuses = serde_json::Map::new();
    for row in &owned {
        let id: i32 = row.get(0);
        statuses.insert(id.to_string(), serde_json::json!("owner"));
    }
    for row in &invite_rows {
        let run_id: i32 = row.get(0);
        let status: String = row.get(1);
        let is_request: bool = row.get(2);
        let label = match (status.as_str(), is_request) {
            ("accepted", _) => "accepted",
            ("pending", false) => "pending_invite",
            ("pending", true) => "pending_request",
            _ => continue,
        };
        statuses.entry(run_id.to_string())
            .or_insert_with(|| serde_json::json!(label));
    }
    serde_json::json!({ "statuses": statuses })
}

/// Return all pending access requests on runs owned by the given user.
///
/// Used by the join-page owner view to approve/deny requests in bulk.
pub fn get_my_run_requests_json(conn_str: &str, owner_id: u32) -> serde_json::Value {
    let mut client = match Client::connect(conn_str, NoTls) {
        Ok(c) => c,
        Err(e) => return serde_json::json!({ "error": format!("DB connection failed: {e}") }),
    };
    let rows = match client.query(
        "SELECT ri.id, ri.run_id, r.player_name, u.id AS uid, u.username, ri.created_at
         FROM run_invites ri
         JOIN runs  r ON r.id  = ri.run_id
         JOIN users u ON u.id  = ri.invited_user
         WHERE ri.is_request = TRUE AND ri.status = 'pending'
           AND r.user_id = $1
         ORDER BY ri.created_at ASC",
        &[&(owner_id as i32)],
    ) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": format!("Query failed: {e}") }),
    };
    let requests: Vec<serde_json::Value> = rows.iter().map(|row| {
        let created: i64 = row.get(5);
        serde_json::json!({
            "invite_id":   row.get::<_, i32>(0),
            "run_id":      row.get::<_, i32>(1),
            "player_name": row.get::<_, String>(2),
            "user_id":     row.get::<_, i32>(3),
            "username":    row.get::<_, String>(4),
            "created_at":  format_timestamp(created as u64),
        })
    }).collect();
    serde_json::json!({ "requests": requests })
}

#[cfg(test)]
mod session_tests {
    use super::*;

    // The DB OnceLock is never initialized in the test process, so a prefix
    // that passes validation returns Ok(false) after the early db() check —
    // these tests exercise only the validation layer.

    #[test]
    fn delete_session_by_prefix_accepts_valid_prefix() {
        assert_eq!(delete_session_by_prefix(1, "0123456789ab"), Ok(false));
    }

    #[test]
    fn delete_session_by_prefix_rejects_bad_input() {
        // Wrong length.
        assert!(delete_session_by_prefix(1, "0123").is_err());
        assert!(delete_session_by_prefix(1, "0123456789abc").is_err());
        assert!(delete_session_by_prefix(1, "").is_err());
        // LIKE wildcards and non-hex characters.
        assert!(delete_session_by_prefix(1, "0123456789a%").is_err());
        assert!(delete_session_by_prefix(1, "0123456789a_").is_err());
        assert!(delete_session_by_prefix(1, "0123456789ag").is_err());
        // Tokens are lowercase hex; uppercase would never match a session.
        assert!(delete_session_by_prefix(1, "0123456789AB").is_err());
    }
}
