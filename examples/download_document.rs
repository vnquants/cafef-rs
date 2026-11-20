//! Download the newest consolidated financial statement of a ticker.
//!
//! Usage:
//!   cargo run -p cafef-rs --example download_document -- FPT ./out

mod common;

use std::path::PathBuf;

use cafef::{CafefClient, DocumentCategory, StatementScope};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    common::init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let symbol = args.first().map_or("FPT", String::as_str).to_uppercase();
    let out_dir = PathBuf::from(args.get(1).map_or("./cafef-downloads", String::as_str));

    let client = CafefClient::new()?;
    let docs = client
        .list_documents(&symbol, DocumentCategory::FinancialStatement, None)
        .await?;
    let doc = docs
        .iter()
        .find(|d| d.title_info().scope == StatementScope::Consolidated)
        .or_else(|| docs.first())
        .ok_or("no financial statements listed")?;

    let path = client.download_document_to_file(doc, &out_dir).await?;
    let size = std::fs::metadata(&path)?.len();
    let magic = std::fs::read(&path)?
        .get(..4)
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();
    info!(
        id = %doc.stable_id(),
        title = %doc.title,
        shown = %doc.time_raw,
        uploaded = ?doc.link_version_time(),
        path = %path.display(),
        size,
        %magic,
        "downloaded"
    );
    Ok(())
}
