use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{ArgGroup, Parser, Subcommand};
use loong_memory_core::{
    AllowAllPolicy, DeterministicHashEmbedder, EngineConfig, MemoryDeleteRequest, MemoryEngine,
    MemoryGetRequest, MemoryPutRequest, OperationContext, RecallRequest, ScoreWeights,
    SqliteAuditSink, SqliteStore, StaticPolicy, StaticPolicyConfig,
};
use serde_json::{json, Value};

#[derive(Parser, Debug)]
#[command(name = "loong-memory")]
#[command(about = "Rust-native memory engine CLI", version)]
struct Cli {
    #[arg(long, global = true)]
    policy_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init(InitCommand),
    Put(PutCommand),
    Get(GetCommand),
    Recall(RecallCommand),
    Delete(DeleteCommand),
    Audit(AuditCommand),
    VectorHealth(VectorHealthCommand),
    VectorRepair(VectorRepairCommand),
}

#[derive(clap::Args, Debug)]
struct InitCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
}

#[derive(clap::Args, Debug)]
struct PutCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long)]
    content: String,
    #[arg(long)]
    external_id: Option<String>,
    #[arg(long, default_value = "{}")]
    metadata: String,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

#[derive(clap::Args, Debug)]
#[command(group(
    ArgGroup::new("selector")
        .required(true)
        .args(["id", "external_id"])
))]
struct GetCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    external_id: Option<String>,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

#[derive(clap::Args, Debug)]
struct RecallCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long)]
    query: String,
    #[arg(long, default_value_t = 5)]
    limit: usize,
    #[arg(long, default_value_t = 0.55)]
    lexical_weight: f32,
    #[arg(long, default_value_t = 0.45)]
    vector_weight: f32,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

#[derive(clap::Args, Debug)]
#[command(group(
    ArgGroup::new("selector")
        .required(true)
        .args(["id", "external_id"])
))]
struct DeleteCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    external_id: Option<String>,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

#[derive(clap::Args, Debug)]
struct AuditCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[arg(long)]
    principal: String,
}

#[derive(clap::Args, Debug)]
struct VectorHealthCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long, default_value_t = 20)]
    invalid_sample_limit: usize,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

#[derive(clap::Args, Debug)]
struct VectorRepairCommand {
    #[arg(long, default_value = "./loong-memory.db")]
    db: PathBuf,
    #[arg(long)]
    namespace: String,
    #[arg(long, default_value_t = 20)]
    issue_sample_limit: usize,
    #[arg(long, default_value_t = false)]
    apply: bool,
    #[arg(long, default_value = "cli-user")]
    principal: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let policy_file = cli.policy_file.clone();

    match cli.command {
        Commands::Init(cmd) => handle_init(cmd),
        Commands::Put(cmd) => handle_put(cmd, policy_file.as_deref()),
        Commands::Get(cmd) => handle_get(cmd, policy_file.as_deref()),
        Commands::Recall(cmd) => handle_recall(cmd, policy_file.as_deref()),
        Commands::Delete(cmd) => handle_delete(cmd, policy_file.as_deref()),
        Commands::Audit(cmd) => handle_audit(cmd, policy_file.as_deref()),
        Commands::VectorHealth(cmd) => handle_vector_health(cmd, policy_file.as_deref()),
        Commands::VectorRepair(cmd) => handle_vector_repair(cmd, policy_file.as_deref()),
    }
}

fn handle_init(cmd: InitCommand) -> Result<()> {
    let _store = SqliteStore::open(&cmd.db)
        .with_context(|| format!("initialize sqlite store at {}", cmd.db.display()))?;
    let _audit = SqliteAuditSink::open(&cmd.db)
        .with_context(|| format!("initialize sqlite audit at {}", cmd.db.display()))?;

    print_json(&json!({
        "ok": true,
        "db": cmd.db,
        "message": "database initialized"
    }))
}

fn handle_put(cmd: PutCommand, policy_file: Option<&Path>) -> Result<()> {
    let metadata = parse_metadata(&cmd.metadata)?;
    let mut engine = open_engine(&cmd.db, policy_file)?;
    let record = engine.put(
        &OperationContext::new(cmd.principal),
        &MemoryPutRequest {
            namespace: cmd.namespace,
            external_id: cmd.external_id,
            content: cmd.content,
            metadata,
        },
    )?;
    print_json(&record)
}

fn handle_get(cmd: GetCommand, policy_file: Option<&Path>) -> Result<()> {
    let engine = open_engine(&cmd.db, policy_file)?;
    let record = engine.get(
        &OperationContext::new(cmd.principal),
        &MemoryGetRequest {
            namespace: cmd.namespace,
            id: cmd.id,
            external_id: cmd.external_id,
        },
    )?;
    print_json(&record)
}

fn handle_recall(cmd: RecallCommand, policy_file: Option<&Path>) -> Result<()> {
    if cmd.limit == 0 {
        bail!("--limit must be greater than 0");
    }
    let weight_sum = cmd.lexical_weight + cmd.vector_weight;
    if weight_sum <= 0.0 {
        bail!("lexical/vector weights must sum to a positive value");
    }

    let engine = open_engine(&cmd.db, policy_file)?;
    let hits = engine.recall(
        &OperationContext::new(cmd.principal),
        &RecallRequest {
            namespace: cmd.namespace,
            query: cmd.query,
            limit: cmd.limit,
            weights: ScoreWeights {
                lexical: cmd.lexical_weight / weight_sum,
                vector: cmd.vector_weight / weight_sum,
            },
        },
    )?;

    print_json(&json!({
        "count": hits.len(),
        "hits": hits
    }))
}

fn handle_delete(cmd: DeleteCommand, policy_file: Option<&Path>) -> Result<()> {
    let mut engine = open_engine(&cmd.db, policy_file)?;
    engine.delete(
        &OperationContext::new(cmd.principal),
        &MemoryDeleteRequest {
            namespace: cmd.namespace,
            id: cmd.id,
            external_id: cmd.external_id,
        },
    )?;

    print_json(&json!({ "ok": true }))
}

fn handle_audit(cmd: AuditCommand, policy_file: Option<&Path>) -> Result<()> {
    let engine = open_engine(&cmd.db, policy_file)?;
    let events = engine.audit_events(
        &OperationContext::new(cmd.principal),
        &cmd.namespace,
        cmd.limit,
    )?;
    print_json(&json!({
        "count": events.len(),
        "events": events
    }))
}

fn handle_vector_health(cmd: VectorHealthCommand, policy_file: Option<&Path>) -> Result<()> {
    let engine = open_engine(&cmd.db, policy_file)?;
    let report = engine.vector_health(
        &OperationContext::new(cmd.principal),
        &cmd.namespace,
        cmd.invalid_sample_limit,
    )?;
    print_json(&report)
}

fn handle_vector_repair(cmd: VectorRepairCommand, policy_file: Option<&Path>) -> Result<()> {
    let mut engine = open_engine(&cmd.db, policy_file)?;
    let report = engine.vector_repair(
        &OperationContext::new(cmd.principal),
        &cmd.namespace,
        cmd.issue_sample_limit,
        cmd.apply,
    )?;
    print_json(&report)
}

fn parse_metadata(metadata_raw: &str) -> Result<Value> {
    let parsed: Value =
        serde_json::from_str(metadata_raw).with_context(|| "parse --metadata JSON")?;
    if !parsed.is_object() {
        bail!("--metadata must be a JSON object");
    }
    Ok(parsed)
}

fn open_engine(db_path: &Path, policy_file: Option<&Path>) -> Result<MemoryEngine<SqliteStore>> {
    let store = SqliteStore::open(db_path)
        .with_context(|| format!("open sqlite store {}", db_path.display()))?;
    let policy = load_policy(policy_file)?;
    let embedder = Arc::new(DeterministicHashEmbedder::default());
    let audit = Arc::new(SqliteAuditSink::open(db_path)?);

    Ok(MemoryEngine::new(
        store,
        policy,
        embedder,
        audit,
        EngineConfig::default(),
    ))
}

fn load_policy(policy_file: Option<&Path>) -> Result<Arc<dyn loong_memory_core::PolicyEngine>> {
    match policy_file {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("read policy file {}", path.display()))?;
            let config: StaticPolicyConfig = serde_json::from_str(&raw)
                .with_context(|| format!("parse policy file {}", path.display()))?;
            Ok(Arc::new(StaticPolicy::from_config(config)))
        }
        None => Ok(Arc::new(AllowAllPolicy)),
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).context("serialize json output")?
    );
    Ok(())
}
