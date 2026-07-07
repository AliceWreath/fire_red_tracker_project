//! Run goals: the user-defined checklist stored in `run_goals`.

use super::*;

// ---------------------------------------------------------------------------
// Run goals — user-defined checklist stored in `run_goals`
// ---------------------------------------------------------------------------

/// A single user-defined goal row.
#[derive(Debug, Clone)]
pub struct GoalRow {
    pub id: i32,
    pub text: String,
    pub completed: bool,
}

/// Creates a new goal for `run_id` and returns its assigned `id`.
///
/// Returns `None` on DB error or when the database is not initialized.
pub fn create_goal(conn_str: &str, run_id: u32, text: &str) -> Option<i32> {
    let conn_str = normalize_conn_str(conn_str);
    let mut client = Client::connect(&conn_str, NoTls).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let row = client
        .query_one(
            "INSERT INTO run_goals (run_id, text, completed, created_at)
             VALUES ($1, $2, FALSE, $3) RETURNING id",
            &[&(run_id as i32), &text, &now],
        )
        .ok()?;
    Some(row.get(0))
}

/// Marks the goal with `goal_id` as completed.  Returns `true` if a row was updated.
pub fn complete_goal(conn_str: &str, goal_id: i32) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "UPDATE run_goals SET completed = TRUE WHERE id = $1",
            &[&goal_id],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Sets the `completed` flag on `goal_id` to `completed`.  Returns `true` if a row was updated.
pub fn set_goal_completed(conn_str: &str, goal_id: i32, completed: bool) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute(
            "UPDATE run_goals SET completed = $2 WHERE id = $1",
            &[&goal_id, &completed],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Deletes the goal with `goal_id`.  Returns `true` if a row was deleted.
pub fn delete_goal(conn_str: &str, goal_id: i32) -> bool {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return false };
    client
        .execute("DELETE FROM run_goals WHERE id = $1", &[&goal_id])
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Returns all goals for `run_id`, ordered by creation time.
pub fn list_goals_for_run(conn_str: &str, run_id: u32) -> Vec<GoalRow> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return vec![] };
    client
        .query(
            "SELECT id, text, completed FROM run_goals WHERE run_id = $1 ORDER BY created_at ASC",
            &[&(run_id as i32)],
        )
        .unwrap_or_default()
        .into_iter()
        .map(|row| GoalRow {
            id:        row.get(0),
            text:      row.get(1),
            completed: row.get(2),
        })
        .collect()
}

/// Returns the `run_id` for the goal with `goal_id`, or `None` if not found.
pub fn get_run_id_for_goal(conn_str: &str, goal_id: i32) -> Option<u32> {
    let conn_str = normalize_conn_str(conn_str);
    let Ok(mut client) = Client::connect(&conn_str, NoTls) else { return None };
    let row = client
        .query_opt("SELECT run_id FROM run_goals WHERE id = $1", &[&goal_id])
        .ok()??;
    Some(row.get::<_, i32>(0) as u32)
}
