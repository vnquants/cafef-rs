//! Fetch one CafeF statement table and log its rows.
//!
//! Usage:
//!   cargo run -p cafef-rs --example get_statement -- FPT cdkt quarter 4
//!   cargo run -p cafef-rs --example get_statement -- VNM kqkd year 2
//!   cargo run -p cafef-rs --example get_statement -- VNM lctt half 1
//!   cargo run -p cafef-rs --example get_statement -- FPT cstc year 3
//!   cargo run -p cafef-rs --example get_statement -- FPT cdkt year all   # whole history
//!
//! Arguments: SYMBOL KIND(cdkt|kqkd|lctt|cstc) PERIOD(quarter|year|half) [N|all] [--summary]
//! `--summary` logs only top-level lines. Money is logged in billions of dong.

mod common;

use cafef::{CafefClient, Decimal, PeriodType, StatementKind, Unit};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    common::init_tracing();
    let all_args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<String> = all_args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let symbol = args.first().map_or("FPT", String::as_str).to_uppercase();
    let kind = args.get(1).map_or("cdkt", String::as_str);
    let kind = StatementKind::parse(kind).ok_or_else(|| format!("unknown table {kind:?}"))?;
    let period = args.get(2).map_or("quarter", String::as_str);
    let period = PeriodType::parse(period).ok_or_else(|| format!("unknown period {period:?}"))?;
    let max_periods = match args.get(3).map(String::as_str) {
        Some("all") => None,
        Some(n) => Some(n.parse::<usize>()?),
        None => Some(4),
    };

    let client = CafefClient::new()?;
    let st = client.statement(&symbol, kind, period, max_periods).await?;

    let labels: Vec<&str> = st.periods.iter().map(|p| p.label.as_str()).collect();
    info!(
        symbol = %st.symbol,
        table = %st.kind,
        periods = ?labels,
        statuses = ?st.periods.iter().map(|p| p.status.as_deref().unwrap_or("-")).collect::<Vec<_>>(),
        report_types = ?st.periods.iter().map(|p| p.report_type.as_deref().unwrap_or("-")).collect::<Vec<_>>(),
        available = ?st.total_periods,
        rows = st.rows.len(),
        complete = st.is_complete(),
        "statement"
    );
    for (period, assets, sources) in st.unbalanced_periods() {
        tracing::warn!(%period, %assets, %sources, diff = %(assets - sources), "assets != liabilities + equity");
    }
    let quiet = all_args.iter().any(|a| a == "--summary");
    let billion = Decimal::from(1_000_000_000_i64);
    for row in &st.rows {
        // The income statement's `level` is not an indentation level; keep all 24 lines.
        if quiet && st.kind != StatementKind::IncomeStatement && row.level.unwrap_or(0) > 1 {
            continue;
        }
        let values: Vec<String> = row
            .values
            .iter()
            .map(|v| match (v.value, row.unit) {
                (None, _) => "-".to_string(),
                // Show money in billions of dong, as the web page does by default.
                (Some(x), Unit::Vnd) => format!("{}", (x / billion).round_dp(2)),
                (Some(x), _) => x.to_string(),
            })
            .collect();
        info!(
            section = row.section_code.as_deref().unwrap_or(""),
            code = %row.code,
            level = row.level.unwrap_or(0),
            unit = ?row.unit,
            name = %row.name,
            values = %values.join(" | "),
            "row"
        );
    }
    Ok(())
}
