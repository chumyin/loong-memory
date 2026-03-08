use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub namespace: String,
    pub external_id: Option<String>,
    pub content: String,
    pub metadata: Value,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPutRequest {
    pub namespace: String,
    pub external_id: Option<String>,
    pub content: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryGetRequest {
    pub namespace: String,
    pub id: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDeleteRequest {
    pub namespace: String,
    pub id: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScoreWeights {
    pub lexical: f32,
    pub vector: f32,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            lexical: 0.55,
            vector: 0.45,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallRequest {
    pub namespace: String,
    pub query: String,
    pub limit: usize,
    pub weights: ScoreWeights,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallHit {
    pub record: MemoryRecord,
    pub lexical_score: f32,
    pub vector_score: f32,
    pub hybrid_score: f32,
}
