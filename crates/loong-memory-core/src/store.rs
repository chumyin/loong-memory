use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{params, types::ValueRef, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    embed::EmbeddingProvider,
    error::LoongMemoryError,
    model::{
        MemoryDeleteRequest, MemoryGetRequest, MemoryPutRequest, MemoryRecord, RecallHit,
        RecallRequest,
    },
    tokenize::tokenize_terms,
};

pub trait MemoryStore {
    fn put(
        &mut self,
        req: &MemoryPutRequest,
        embedder: &dyn EmbeddingProvider,
    ) -> Result<MemoryRecord, LoongMemoryError>;

    fn get(&self, req: &MemoryGetRequest) -> Result<MemoryRecord, LoongMemoryError>;

    fn delete(&mut self, req: &MemoryDeleteRequest) -> Result<(), LoongMemoryError>;

    fn recall(
        &self,
        req: &RecallRequest,
        embedder: &dyn EmbeddingProvider,
    ) -> Result<Vec<RecallHit>, LoongMemoryError>;
}

pub struct SqliteStore {
    conn: Connection,
    _path: PathBuf,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LoongMemoryError> {
        let path_ref = path.as_ref().to_path_buf();
        let mut conn = Connection::open(&path_ref).map_err(storage_err("open sqlite database"))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(storage_err("set journal_mode=WAL"))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(storage_err("set synchronous=NORMAL"))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(storage_err("set foreign_keys=ON"))?;
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(storage_err("set temp_store=MEMORY"))?;
        conn.busy_timeout(Duration::from_millis(5_000))
            .map_err(storage_err("set busy_timeout"))?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_migrations(version, applied_at)
            VALUES (1, datetime('now'));

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                external_id TEXT,
                content TEXT NOT NULL,
                metadata TEXT NOT NULL CHECK(json_valid(metadata)),
                content_hash TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_memories_namespace_external
                ON memories(namespace, external_id)
                WHERE external_id IS NOT NULL;

            CREATE INDEX IF NOT EXISTS idx_memories_namespace_updated
                ON memories(namespace, updated_at DESC);

            CREATE TABLE IF NOT EXISTS memory_vectors (
                memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
                dimension INTEGER NOT NULL,
                vector BLOB NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                content,
                namespace UNINDEXED,
                memory_id UNINDEXED,
                tokenize='unicode61 remove_diacritics 2'
            );

            CREATE TABLE IF NOT EXISTS memory_audit (
                event_id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                principal TEXT NOT NULL,
                namespace TEXT NOT NULL,
                action TEXT NOT NULL,
                kind TEXT NOT NULL,
                detail TEXT NOT NULL
            );
            "#,
        )
        .map_err(storage_err("initialize schema"))?;
        Self::apply_schema_migration_v2_legacy_text_vectors(&mut conn)?;

        Ok(Self {
            conn,
            _path: path_ref,
        })
    }

    fn is_schema_version_applied(
        conn: &Connection,
        version: i64,
    ) -> Result<bool, LoongMemoryError> {
        let applied = conn
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = ?1 LIMIT 1",
                params![version],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(storage_err("query schema migration version"))?;
        Ok(applied.is_some())
    }

    fn apply_schema_migration_v2_legacy_text_vectors(
        conn: &mut Connection,
    ) -> Result<(), LoongMemoryError> {
        if Self::is_schema_version_applied(conn, 2)? {
            return Ok(());
        }

        let tx = conn
            .transaction()
            .map_err(storage_err("start schema migration v2 transaction"))?;
        let mut stmt = tx
            .prepare(
                r#"
                SELECT memory_id, vector
                FROM memory_vectors
                WHERE typeof(vector) = 'text'
                "#,
            )
            .map_err(storage_err("prepare schema migration v2 scan"))?;
        let mut rows = stmt
            .query([])
            .map_err(storage_err("query schema migration v2 scan"))?;

        let mut legacy_rows: Vec<(String, String)> = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(storage_err("scan schema migration v2 rows"))?
        {
            let memory_id: String = row
                .get(0)
                .map_err(storage_err("read schema migration v2 memory_id"))?;
            let vector_text: String = row
                .get(1)
                .map_err(storage_err("read schema migration v2 vector text"))?;
            legacy_rows.push((memory_id, vector_text));
        }
        drop(rows);
        drop(stmt);

        for (memory_id, vector_text) in legacy_rows {
            let Ok(vector) = serde_json::from_str::<Vec<f32>>(&vector_text) else {
                continue;
            };
            if vector.is_empty() || !vector.iter().all(|v| v.is_finite()) {
                continue;
            }
            tx.execute(
                "UPDATE memory_vectors SET dimension = ?1, vector = ?2 WHERE memory_id = ?3",
                params![vector.len() as i64, encode_vector_blob(&vector), memory_id],
            )
            .map_err(storage_err("update schema migration v2 vector row"))?;
        }

        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, datetime('now'))",
            [],
        )
        .map_err(storage_err("record schema migration v2"))?;
        tx.commit()
            .map_err(storage_err("commit schema migration v2 transaction"))?;
        Ok(())
    }

    fn fetch_by_id_and_namespace(
        conn: &Connection,
        namespace: &str,
        id: &str,
    ) -> Result<Option<MemoryRecord>, LoongMemoryError> {
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, namespace, external_id, content, metadata, content_hash, created_at, updated_at
                FROM memories
                WHERE namespace = ?1 AND id = ?2
                LIMIT 1
                "#,
            )
            .map_err(storage_err("prepare get by id"))?;
        let row = stmt
            .query_row(params![namespace, id], row_to_memory_record)
            .optional()
            .map_err(storage_err("query get by id"))?;
        Ok(row)
    }

    fn fetch_by_external_id(
        conn: &Connection,
        namespace: &str,
        external_id: &str,
    ) -> Result<Option<MemoryRecord>, LoongMemoryError> {
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, namespace, external_id, content, metadata, content_hash, created_at, updated_at
                FROM memories
                WHERE namespace = ?1 AND external_id = ?2
                LIMIT 1
                "#,
            )
            .map_err(storage_err("prepare get by external_id"))?;
        let row = stmt
            .query_row(params![namespace, external_id], row_to_memory_record)
            .optional()
            .map_err(storage_err("query get by external_id"))?;
        Ok(row)
    }

    fn selector_from_request<'a>(
        namespace: &'a str,
        id: &'a Option<String>,
        external_id: &'a Option<String>,
    ) -> Result<MemorySelector<'a>, LoongMemoryError> {
        if namespace.trim().is_empty() {
            return Err(LoongMemoryError::Validation(
                "namespace is required".to_owned(),
            ));
        }
        match (id.as_deref(), external_id.as_deref()) {
            (Some(_), Some(_)) => Err(LoongMemoryError::Validation(
                "choose either id or external_id, not both".to_owned(),
            )),
            (None, None) => Err(LoongMemoryError::Validation(
                "id or external_id is required".to_owned(),
            )),
            (Some(memory_id), None) => Ok(MemorySelector::ById(memory_id)),
            (None, Some(ext)) if ext.trim().is_empty() => Err(LoongMemoryError::Validation(
                "external_id cannot be empty".to_owned(),
            )),
            (None, Some(ext)) => Ok(MemorySelector::ByExternalId(ext)),
        }
    }

    fn update_aux_indexes(
        conn: &Connection,
        memory_id: &str,
        namespace: &str,
        content: &str,
        vector: &[f32],
    ) -> Result<(), LoongMemoryError> {
        conn.execute(
            "DELETE FROM memory_fts WHERE memory_id = ?1",
            params![memory_id],
        )
        .map_err(storage_err("delete previous fts index"))?;
        conn.execute(
            "INSERT INTO memory_fts(content, namespace, memory_id) VALUES (?1, ?2, ?3)",
            params![content, namespace, memory_id],
        )
        .map_err(storage_err("insert fts index"))?;

        let vector_blob = encode_vector_blob(vector);
        conn.execute(
            r#"
            INSERT INTO memory_vectors(memory_id, dimension, vector)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(memory_id) DO UPDATE SET
                dimension = excluded.dimension,
                vector = excluded.vector
            "#,
            params![memory_id, vector.len() as i64, vector_blob],
        )
        .map_err(storage_err("upsert vector row"))?;

        Ok(())
    }

    fn build_fts_query(query: &str) -> Option<String> {
        let mut terms = Vec::new();
        for token in tokenize_terms(query, 24) {
            let escaped = token.replace('"', "\"\"");
            terms.push(format!("\"{escaped}\""));
        }
        if terms.is_empty() {
            None
        } else {
            Some(terms.join(" OR "))
        }
    }

    fn read_vector_candidates(
        &self,
        namespace: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<HashMap<String, f32>, LoongMemoryError> {
        let scan_limit = (limit.max(8) * 8).clamp(32, 512) as i64;
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT mv.memory_id, mv.dimension, mv.vector
                FROM memory_vectors mv
                JOIN memories m ON m.id = mv.memory_id
                WHERE m.namespace = ?1
                ORDER BY m.updated_at DESC
                LIMIT ?2
                "#,
            )
            .map_err(storage_err("prepare vector recall query"))?;
        let mut rows = stmt
            .query(params![namespace, scan_limit])
            .map_err(storage_err("query vector candidates"))?;

        let mut out = HashMap::new();
        while let Some(row) = rows.next().map_err(storage_err("scan vector candidates"))? {
            let id: String = row
                .get(0)
                .map_err(storage_err("read vector candidate memory_id"))?;
            let stored_dimension: i64 = row
                .get(1)
                .map_err(storage_err("read vector candidate dimension"))?;
            let Ok(stored_dimension) = usize::try_from(stored_dimension) else {
                continue;
            };
            if stored_dimension != query_vector.len() {
                continue;
            }
            let value_ref = row
                .get_ref(2)
                .map_err(storage_err("read vector candidate value"))?;
            let candidate = match decode_vector_value(value_ref, &id, stored_dimension) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !candidate.iter().all(|v| v.is_finite()) {
                continue;
            }
            let cosine = cosine_similarity(query_vector, &candidate);
            if !cosine.is_finite() {
                continue;
            }
            let normalized = ((cosine + 1.0) / 2.0).clamp(0.0, 1.0);
            out.insert(id, normalized);
        }
        Ok(out)
    }

    fn read_lexical_candidates(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<HashMap<String, f32>, LoongMemoryError> {
        let mut out = HashMap::new();
        let Some(fts_query) = Self::build_fts_query(query) else {
            return Ok(out);
        };

        let lexical_limit = (limit.max(8) * 8).clamp(32, 512) as i64;
        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT memory_id, bm25(memory_fts) AS rank
                FROM memory_fts
                WHERE namespace = ?1 AND memory_fts MATCH ?2
                ORDER BY rank ASC
                LIMIT ?3
                "#,
            )
            .map_err(storage_err("prepare lexical recall query"))?;
        let mut rows = stmt
            .query(params![namespace, fts_query, lexical_limit])
            .map_err(storage_err("query lexical recall candidates"))?;

        while let Some(row) = rows
            .next()
            .map_err(storage_err("scan lexical recall candidates"))?
        {
            let memory_id: String = row
                .get(0)
                .map_err(storage_err("read lexical candidate memory_id"))?;
            let rank: f64 = row.get(1).unwrap_or(0.0);
            let score = (1.0 / (1.0 + rank.abs() as f32)).clamp(0.0, 1.0);
            out.insert(memory_id, score);
        }

        if out.is_empty() {
            return self.read_lexical_fallback_candidates(namespace, query, lexical_limit as usize);
        }

        Ok(out)
    }

    fn read_lexical_fallback_candidates(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<HashMap<String, f32>, LoongMemoryError> {
        let query_terms: HashSet<String> = tokenize_terms(query, 32).into_iter().collect();
        if query_terms.is_empty() {
            return Ok(HashMap::new());
        }

        let mut stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, content
                FROM memories
                WHERE namespace = ?1
                ORDER BY updated_at DESC
                LIMIT ?2
                "#,
            )
            .map_err(storage_err("prepare lexical fallback query"))?;
        let mut rows = stmt
            .query(params![namespace, limit as i64])
            .map_err(storage_err("query lexical fallback candidates"))?;

        let mut out = HashMap::new();
        while let Some(row) = rows
            .next()
            .map_err(storage_err("scan lexical fallback candidates"))?
        {
            let memory_id: String = row
                .get(0)
                .map_err(storage_err("read lexical fallback memory_id"))?;
            let content: String = row
                .get(1)
                .map_err(storage_err("read lexical fallback content"))?;
            let content_terms: HashSet<String> =
                tokenize_terms(&content, 128).into_iter().collect();
            if content_terms.is_empty() {
                continue;
            }
            let overlap = query_terms.intersection(&content_terms).count();
            if overlap == 0 {
                continue;
            }
            let score = (overlap as f32 / query_terms.len() as f32).clamp(0.0, 1.0);
            out.insert(memory_id, score);
        }

        Ok(out)
    }
}

enum MemorySelector<'a> {
    ById(&'a str),
    ByExternalId(&'a str),
}

fn row_to_memory_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let created_at_raw: String = row.get("created_at")?;
    let updated_at_raw: String = row.get("updated_at")?;
    let metadata_raw: String = row.get("metadata")?;

    let created_at = DateTime::parse_from_rfc3339(&created_at_raw)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?
        .with_timezone(&Utc);
    let updated_at = DateTime::parse_from_rfc3339(&updated_at_raw)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?
        .with_timezone(&Utc);
    let metadata = serde_json::from_str::<Value>(&metadata_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(MemoryRecord {
        id: row.get("id")?,
        namespace: row.get("namespace")?,
        external_id: row.get("external_id")?,
        content: row.get("content")?,
        metadata,
        content_hash: row.get("content_hash")?,
        created_at,
        updated_at,
    })
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

fn encode_vector_blob(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn decode_vector_blob(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "invalid blob size {}, not divisible by 4",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn decode_vector_value(
    value_ref: ValueRef<'_>,
    memory_id: &str,
    expected_dimension: usize,
) -> Result<Vec<f32>, LoongMemoryError> {
    let vector = match value_ref {
        ValueRef::Blob(bytes) => decode_vector_blob(bytes).map_err(|e| {
            LoongMemoryError::Storage(format!(
                "decode blob vector for memory {memory_id} failed: {e}"
            ))
        }),
        ValueRef::Text(text_bytes) => {
            let text = str::from_utf8(text_bytes).map_err(|e| {
                LoongMemoryError::Storage(format!(
                    "decode text vector utf8 for memory {memory_id} failed: {e}"
                ))
            })?;
            serde_json::from_str::<Vec<f32>>(text).map_err(|e| {
                LoongMemoryError::Storage(format!(
                    "parse text vector json for memory {memory_id} failed: {e}"
                ))
            })
        }
        ValueRef::Null => Err(LoongMemoryError::Storage(format!(
            "vector value for memory {memory_id} is null"
        ))),
        other => Err(LoongMemoryError::Storage(format!(
            "vector value for memory {memory_id} has unsupported sqlite type: {other:?}"
        ))),
    }?;

    if vector.len() != expected_dimension {
        return Err(LoongMemoryError::Storage(format!(
            "vector dimension mismatch for memory {memory_id}: decoded={} expected={expected_dimension}",
            vector.len()
        )));
    }
    if !vector.iter().all(|v| v.is_finite()) {
        return Err(LoongMemoryError::Storage(format!(
            "vector contains non-finite values for memory {memory_id}"
        )));
    }

    Ok(vector)
}

fn storage_err(
    stage: &'static str,
) -> impl Fn(rusqlite::Error) -> LoongMemoryError + Copy + 'static {
    move |e| LoongMemoryError::Storage(format!("{stage}: {e}"))
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let dim = a.len().min(b.len());
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for idx in 0..dim {
        dot += a[idx] * b[idx];
        norm_a += a[idx] * a[idx];
        norm_b += b[idx] * b[idx];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

impl MemoryStore for SqliteStore {
    fn put(
        &mut self,
        req: &MemoryPutRequest,
        embedder: &dyn EmbeddingProvider,
    ) -> Result<MemoryRecord, LoongMemoryError> {
        if req.namespace.trim().is_empty() {
            return Err(LoongMemoryError::Validation(
                "namespace is required".to_owned(),
            ));
        }
        if let Some(external_id) = req.external_id.as_deref() {
            if external_id.trim().is_empty() {
                return Err(LoongMemoryError::Validation(
                    "external_id cannot be empty".to_owned(),
                ));
            }
        }
        if !req.metadata.is_object() {
            return Err(LoongMemoryError::Validation(
                "metadata must be a json object".to_owned(),
            ));
        }

        let vector = embedder.embed(&req.content)?;
        let metadata_json = serde_json::to_string(&req.metadata)
            .map_err(|e| LoongMemoryError::Storage(format!("serialize metadata: {e}")))?;
        let content_hash = hash_content(&req.content);
        let now_rfc3339 = Utc::now().to_rfc3339();

        let tx = self
            .conn
            .transaction()
            .map_err(storage_err("start put transaction"))?;

        let existing = if let Some(external_id) = req.external_id.as_deref() {
            let mut stmt = tx
                .prepare(
                    r#"
                    SELECT id
                    FROM memories
                    WHERE namespace = ?1 AND external_id = ?2
                    LIMIT 1
                    "#,
                )
                .map_err(storage_err("prepare existing memory lookup"))?;
            stmt.query_row(params![req.namespace, external_id], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(storage_err("query existing memory by external_id"))?
        } else {
            None
        };

        let memory_id = existing
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        if existing.is_some() {
            tx.execute(
                r#"
                UPDATE memories
                SET content = ?1,
                    metadata = ?2,
                    content_hash = ?3,
                    updated_at = ?4
                WHERE id = ?5
                "#,
                params![
                    req.content,
                    metadata_json,
                    content_hash,
                    now_rfc3339,
                    memory_id.clone()
                ],
            )
            .map_err(storage_err("update memory row"))?;
        } else {
            tx.execute(
                r#"
                INSERT INTO memories(
                    id, namespace, external_id, content, metadata, content_hash, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    memory_id.clone(),
                    req.namespace,
                    req.external_id,
                    req.content,
                    metadata_json,
                    content_hash,
                    now_rfc3339,
                    now_rfc3339
                ],
            )
            .map_err(storage_err("insert memory row"))?;
        }

        Self::update_aux_indexes(&tx, &memory_id, &req.namespace, &req.content, &vector)?;
        tx.commit().map_err(storage_err("commit put transaction"))?;

        Self::fetch_by_id_and_namespace(&self.conn, &req.namespace, &memory_id)?
            .ok_or(LoongMemoryError::NotFound)
    }

    fn get(&self, req: &MemoryGetRequest) -> Result<MemoryRecord, LoongMemoryError> {
        match Self::selector_from_request(&req.namespace, &req.id, &req.external_id)? {
            MemorySelector::ById(id) => {
                Self::fetch_by_id_and_namespace(&self.conn, &req.namespace, id)?
                    .ok_or(LoongMemoryError::NotFound)
            }
            MemorySelector::ByExternalId(external_id) => {
                Self::fetch_by_external_id(&self.conn, &req.namespace, external_id)?
                    .ok_or(LoongMemoryError::NotFound)
            }
        }
    }

    fn delete(&mut self, req: &MemoryDeleteRequest) -> Result<(), LoongMemoryError> {
        let selector = Self::selector_from_request(&req.namespace, &req.id, &req.external_id)?;
        let tx = self
            .conn
            .transaction()
            .map_err(storage_err("start delete transaction"))?;

        let target_id: Option<String> = match selector {
            MemorySelector::ById(id) => tx
                .query_row(
                    "SELECT id FROM memories WHERE namespace = ?1 AND id = ?2 LIMIT 1",
                    params![req.namespace, id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage_err("lookup memory by id for delete"))?,
            MemorySelector::ByExternalId(external_id) => tx
                .query_row(
                    "SELECT id FROM memories WHERE namespace = ?1 AND external_id = ?2 LIMIT 1",
                    params![req.namespace, external_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage_err("lookup memory by external_id for delete"))?,
        };

        let Some(target_id) = target_id else {
            return Err(LoongMemoryError::NotFound);
        };

        tx.execute(
            "DELETE FROM memory_fts WHERE memory_id = ?1",
            params![target_id.as_str()],
        )
        .map_err(storage_err("delete fts row"))?;

        let affected = tx
            .execute(
                "DELETE FROM memories WHERE namespace = ?1 AND id = ?2",
                params![req.namespace, target_id.as_str()],
            )
            .map_err(storage_err("delete memory row"))?;

        if affected == 0 {
            return Err(LoongMemoryError::NotFound);
        }

        tx.commit()
            .map_err(storage_err("commit delete transaction"))?;
        Ok(())
    }

    fn recall(
        &self,
        req: &RecallRequest,
        embedder: &dyn EmbeddingProvider,
    ) -> Result<Vec<RecallHit>, LoongMemoryError> {
        if req.namespace.trim().is_empty() {
            return Err(LoongMemoryError::Validation(
                "namespace is required".to_owned(),
            ));
        }
        if req.query.trim().is_empty() {
            return Err(LoongMemoryError::Validation("query is required".to_owned()));
        }
        if req.limit == 0 {
            return Err(LoongMemoryError::Validation("limit must be > 0".to_owned()));
        }

        let query_vector = embedder.embed(&req.query)?;
        let lexical_scores = self.read_lexical_candidates(&req.namespace, &req.query, req.limit)?;
        let vector_scores =
            self.read_vector_candidates(&req.namespace, &query_vector, req.limit)?;

        let mut candidate_ids: BTreeSet<String> = BTreeSet::new();
        candidate_ids.extend(lexical_scores.keys().cloned());
        candidate_ids.extend(vector_scores.keys().cloned());

        if candidate_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut hits = Vec::new();
        let mut fetch_stmt = self
            .conn
            .prepare(
                r#"
                SELECT id, namespace, external_id, content, metadata, content_hash, created_at, updated_at
                FROM memories
                WHERE namespace = ?1 AND id = ?2
                LIMIT 1
                "#,
            )
            .map_err(storage_err("prepare recall record fetch"))?;
        for id in candidate_ids {
            let record = fetch_stmt
                .query_row(params![req.namespace, id.as_str()], row_to_memory_record)
                .optional()
                .map_err(storage_err("query recall record fetch"))?;
            let Some(record) = record else {
                continue;
            };
            let lexical = lexical_scores.get(&id).copied().unwrap_or(0.0);
            let vector = vector_scores.get(&id).copied().unwrap_or(0.0);
            let hybrid = req.weights.lexical * lexical + req.weights.vector * vector;
            hits.push(RecallHit {
                record,
                lexical_score: lexical,
                vector_score: vector,
                hybrid_score: hybrid,
            });
        }

        hits.sort_by(|a, b| {
            b.hybrid_score
                .partial_cmp(&a.hybrid_score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.record.updated_at.cmp(&a.record.updated_at))
                .then_with(|| a.record.id.cmp(&b.record.id))
        });
        hits.truncate(req.limit);
        Ok(hits)
    }
}
