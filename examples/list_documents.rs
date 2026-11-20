//! List a ticker's CafeF document library and summarise it.
//!
//! Usage:
//!   cargo run -p cafef-rs --example list_documents -- FPT              # financial statements tab
//!   cargo run -p cafef-rs --example list_documents -- FPT --all        # every tab (5 requests)
//!   cargo run -p cafef-rs --example list_documents -- VNM --year 2025  # one year, statements tab

mod common;

use std::collections::BTreeMap;

use cafef::{Assurance, CafefClient, DocumentCategory, StatementScope};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    common::init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let symbol = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .map_or("FPT", String::as_str)
        .to_uppercase();
    let all = args.iter().any(|a| a == "--all");
    let year = common::flag_value(&args, "--year")
        .map(|y| y.parse::<i32>())
        .transpose()?;

    let client = CafefClient::new()?;
    let docs = if all {
        client.list_all_documents(&symbol).await?
    } else {
        client
            .list_documents(&symbol, DocumentCategory::FinancialStatement, year)
            .await?
    };

    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_ext: BTreeMap<String, usize> = BTreeMap::new();
    for d in &docs {
        *by_category.entry(d.category.to_string()).or_default() += 1;
        *by_ext
            .entry(d.file_ext.clone().unwrap_or_else(|| "?".into()))
            .or_default() += 1;
    }
    let mut years: Vec<i32> = docs.iter().filter_map(|d| d.period_year()).collect();
    years.sort_unstable();
    years.dedup();

    let statements: Vec<_> = docs.iter().filter(|d| d.is_financial_statement()).collect();
    let consolidated = statements
        .iter()
        .filter(|d| d.title_info().scope == StatementScope::Consolidated)
        .count();
    let parent = statements
        .iter()
        .filter(|d| d.title_info().scope == StatementScope::Parent)
        .count();
    let audited = statements
        .iter()
        .filter(|d| d.title_info().assurance == Assurance::Audited)
        .count();
    let reviewed = statements
        .iter()
        .filter(|d| d.title_info().assurance == Assurance::Reviewed)
        .count();
    let without_id = docs.iter().filter(|d| d.cafef_id.is_none()).count();
    let mut fs_years: Vec<i32> = statements.iter().filter_map(|d| d.period_year()).collect();
    fs_years.sort_unstable();
    fs_years.dedup();
    let mut fs_ext: BTreeMap<String, usize> = BTreeMap::new();
    for d in &statements {
        *fs_ext
            .entry(d.file_ext.clone().unwrap_or_else(|| "?".into()))
            .or_default() += 1;
    }

    info!(
        %symbol,
        total = docs.len(),
        financial_statements = statements.len(),
        consolidated,
        parent,
        audited,
        reviewed,
        fs_first_year = fs_years.first(),
        fs_last_year = fs_years.last(),
        ?fs_ext,
        without_cafef_id = without_id,
        first_year = years.first(),
        last_year = years.last(),
        distinct_years = years.len(),
        ?by_category,
        ?by_ext,
        "document library"
    );

    for d in docs.iter().take(8) {
        info!(
            id = %d.stable_id(),
            category = %d.category,
            shown = %d.time_raw,
            ext = d.file_ext.as_deref().unwrap_or("?"),
            title = %d.title,
            url = %d.url,
            "document"
        );
    }
    Ok(())
}
