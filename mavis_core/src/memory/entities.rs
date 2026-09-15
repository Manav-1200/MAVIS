// mavis_core/src/memory/entities.rs
// Entity graph: the projects, applications and files the user actually
// works with, and which of them appear together.
//
// The phase plan specified spaCy for named-entity recognition. Not used
// here, for two reasons: it's a heavy dependency plus a language model in
// the Python worker, and it's poor at exactly the entities that matter on a
// developer's desktop — it won't tag `mavis_core` as a project or `stt.rs`
// as a file, because those aren't the entity types it was trained on.
//
// Instead entities come from data the context layer already resolves with
// certainty: git repository names, application IDs from the compositor, and
// filenames parsed out of IDE window titles. These are observations, not
// predictions — nothing is inferred, so nothing is wrong.
//
// The trade-off worth naming: people mentioned in conversation ("meeting
// with Sarah") are not captured. That's the gap spaCy would fill.

#![allow(dead_code)]

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub enum EntityKind {
    Project,
    App,
    File,
}

impl EntityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Project => "project",
            EntityKind::App => "app",
            EntityKind::File => "file",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub kind: String,
    pub mention_count: i64,
    pub last_seen: String,
}

pub struct EntityStore {
    conn: Connection,
}

impl EntityStore {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS entities (
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                mention_count INTEGER NOT NULL DEFAULT 1,
                first_seen TEXT NOT NULL,
                last_seen TEXT NOT NULL,
                PRIMARY KEY (name, kind)
            )",
            [],
        )?;
        // Co-occurrence: which entities are observed at the same moment.
        // Ordered pairs are normalised so (a,b) and (b,a) are one row.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS entity_links (
                a_name TEXT NOT NULL,
                a_kind TEXT NOT NULL,
                b_name TEXT NOT NULL,
                b_kind TEXT NOT NULL,
                weight INTEGER NOT NULL DEFAULT 1,
                last_seen TEXT NOT NULL,
                PRIMARY KEY (a_name, a_kind, b_name, b_kind)
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn observe(&self, name: &str, kind: EntityKind, when: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name == "unknown" {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO entities (name, kind, mention_count, first_seen, last_seen)
             VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(name, kind) DO UPDATE SET
                mention_count = mention_count + 1,
                last_seen = ?3",
            params![name, kind.as_str(), when],
        )?;
        Ok(())
    }

    /// Record that two entities were observed together. Pairs are sorted so
    /// the same relationship never produces two rows.
    pub fn link(
        &self,
        a: (&str, EntityKind),
        b: (&str, EntityKind),
        when: &str,
    ) -> Result<()> {
        let (a_name, a_kind) = (a.0.trim(), a.1.as_str());
        let (b_name, b_kind) = (b.0.trim(), b.1.as_str());
        if a_name.is_empty() || b_name.is_empty() {
            return Ok(());
        }
        if a_name == b_name && a_kind == b_kind {
            return Ok(());
        }

        let ((x_name, x_kind), (y_name, y_kind)) = if (a_kind, a_name) <= (b_kind, b_name) {
            ((a_name, a_kind), (b_name, b_kind))
        } else {
            ((b_name, b_kind), (a_name, a_kind))
        };

        self.conn.execute(
            "INSERT INTO entity_links (a_name, a_kind, b_name, b_kind, weight, last_seen)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(a_name, a_kind, b_name, b_kind) DO UPDATE SET
                weight = weight + 1,
                last_seen = ?5",
            params![x_name, x_kind, y_name, y_kind, when],
        )?;
        Ok(())
    }

    /// Most-worked-with entities of a kind, by how often they're seen.
    pub fn top(&self, kind: EntityKind, limit: usize) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, kind, mention_count, last_seen FROM entities
             WHERE kind = ?1
             ORDER BY mention_count DESC, last_seen DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![kind.as_str(), limit as i64], |row| {
            Ok(Entity {
                name: row.get(0)?,
                kind: row.get(1)?,
                mention_count: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    /// Entities most often seen alongside this one — "what do I usually have
    /// open when I'm in the MAVIS project".
    pub fn related(&self, name: &str, kind: EntityKind, limit: usize) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT other_name, other_kind, weight, last_seen FROM (
                SELECT b_name AS other_name, b_kind AS other_kind, weight, last_seen
                FROM entity_links WHERE a_name = ?1 AND a_kind = ?2
                UNION ALL
                SELECT a_name AS other_name, a_kind AS other_kind, weight, last_seen
                FROM entity_links WHERE b_name = ?1 AND b_kind = ?2
             )
             ORDER BY weight DESC, last_seen DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![name, kind.as_str(), limit as i64], |row| {
            Ok(Entity {
                name: row.get(0)?,
                kind: row.get(1)?,
                mention_count: row.get(2)?,
                last_seen: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.into())
    }

    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))?;
        Ok(n)
    }
}