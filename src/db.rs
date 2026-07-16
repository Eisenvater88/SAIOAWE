use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

use crate::models::{NodeRun, Run};

/// Simple JSON-document store on SQLite. Each entity lives in one row as a
/// JSON blob; a few extra columns exist where ordering/filtering is needed.
pub struct Db {
    conn: Mutex<Connection>,
}

const ENTITY_TABLES: &[&str] = &["agent_cards", "workflows", "mcp_servers", "schedules"];

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        for t in ENTITY_TABLES {
            conn.execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {t} (id TEXT PRIMARY KEY, json TEXT NOT NULL)"
                ),
                [],
            )?;
        }
        conn.execute(
            "CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                json TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_runs_wf ON runs(workflow_id, started_at)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS node_runs (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                json TEXT NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_node_runs ON node_runs(run_id)",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn check_table(table: &str) -> Result<()> {
        if ENTITY_TABLES.contains(&table) {
            Ok(())
        } else {
            Err(anyhow!("unknown table {table}"))
        }
    }

    pub fn put<T: Serialize>(&self, table: &str, id: &str, value: &T) -> Result<()> {
        Self::check_table(table)?;
        let json = serde_json::to_string(value)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {table} (id, json) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json"
            ),
            params![id, json],
        )?;
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, table: &str, id: &str) -> Result<Option<T>> {
        Self::check_table(table)?;
        let conn = self.conn.lock().unwrap();
        let json: Option<String> = conn
            .query_row(
                &format!("SELECT json FROM {table} WHERE id = ?1"),
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        match json {
            Some(j) => Ok(Some(serde_json::from_str(&j)?)),
            None => Ok(None),
        }
    }

    pub fn list<T: DeserializeOwned>(&self, table: &str) -> Result<Vec<T>> {
        Self::check_table(table)?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT json FROM {table}"))?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn delete(&self, table: &str, id: &str) -> Result<bool> {
        Self::check_table(table)?;
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(&format!("DELETE FROM {table} WHERE id = ?1"), params![id])?;
        Ok(n > 0)
    }

    // ---- runs ----

    pub fn put_run(&self, run: &Run) -> Result<()> {
        let json = serde_json::to_string(run)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO runs (id, workflow_id, started_at, json) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![run.id, run.workflow_id, run.started_at, json],
        )?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<Run>> {
        let conn = self.conn.lock().unwrap();
        let json: Option<String> = conn
            .query_row("SELECT json FROM runs WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .optional()?;
        match json {
            Some(j) => Ok(Some(serde_json::from_str(&j)?)),
            None => Ok(None),
        }
    }

    pub fn list_runs(&self, workflow_id: Option<&str>, limit: u32) -> Result<Vec<Run>> {
        let conn = self.conn.lock().unwrap();
        let mut out = Vec::new();
        match workflow_id {
            Some(wf) => {
                let mut stmt = conn.prepare(
                    "SELECT json FROM runs WHERE workflow_id = ?1
                     ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![wf, limit], |r| r.get::<_, String>(0))?;
                for row in rows {
                    out.push(serde_json::from_str(&row?)?);
                }
            }
            None => {
                let mut stmt =
                    conn.prepare("SELECT json FROM runs ORDER BY started_at DESC LIMIT ?1")?;
                let rows = stmt.query_map(params![limit], |r| r.get::<_, String>(0))?;
                for row in rows {
                    out.push(serde_json::from_str(&row?)?);
                }
            }
        }
        Ok(out)
    }

    pub fn put_node_run(&self, nr: &NodeRun) -> Result<()> {
        let json = serde_json::to_string(nr)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO node_runs (id, run_id, json) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![nr.id, nr.run_id, json],
        )?;
        Ok(())
    }

    pub fn node_runs_for(&self, run_id: &str) -> Result<Vec<NodeRun>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT json FROM node_runs WHERE run_id = ?1")?;
        let rows = stmt.query_map(params![run_id], |r| r.get::<_, String>(0))?;
        let mut out: Vec<NodeRun> = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        out.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        Ok(out)
    }

    /// Runs live in a detached async task; if the process dies mid-run the row
    /// is left as "running" forever. At startup, mark any such run (and its
    /// in-flight node runs) as "interrupted" so history is honest and the
    /// scheduler is not confused by ghost runs. Deliberately does NOT auto
    /// resume: a node may already have caused side effects (sent an e-mail,
    /// created a calendar entry) and re-running it would repeat them.
    pub fn reconcile_interrupted(&self) -> Result<usize> {
        let mut running: Vec<Run> = self.list_runs(None, 100_000)?;
        running.retain(|r| r.status == "running" || r.status == "pending");
        let count = running.len();
        let now = crate::models::now_rfc3339();
        for mut run in running {
            for mut nr in self.node_runs_for(&run.id)? {
                if nr.status == "running" || nr.status == "pending" {
                    nr.status = "interrupted".into();
                    nr.error = Some("process restarted before this agent finished".into());
                    nr.finished_at.get_or_insert_with(|| now.clone());
                    self.put_node_run(&nr)?;
                }
            }
            run.status = "interrupted".into();
            run.error = Some(
                "the server was restarted while this run was in progress; \
                 it was not resumed (re-run it manually if needed)"
                    .into(),
            );
            run.finished_at.get_or_insert_with(|| now.clone());
            self.put_run(&run)?;
        }
        Ok(count)
    }
}
