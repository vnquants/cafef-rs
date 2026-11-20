//! # CafeF client
//!
//! Anonymous, read-only access to two parts of [CafeF](https://cafef.vn)'s
//! company data:
//!
//! - the **document library** ("Tài liệu" page): financial-statement PDFs,
//!   annual reports, prospectuses, resolutions and governance reports, with
//!   download links ([`CafefClient::list_documents`],
//!   [`CafefClient::list_all_documents`], [`CafefClient::download_document`]);
//! - the **statement tables** ("Tài chính" page): balance sheet, income
//!   statement, cash flow (quarterly, annual, first-half cumulative) and annual
//!   ratios ([`CafefClient::statement`] and shortcuts), with values as
//!   [`rust_decimal::Decimal`] in the [`Unit`] CafeF sends (dong).
//!
//! Requests go through [`wreq`] with a Chrome TLS/HTTP2 profile and are spaced
//! at least one second apart by default. See the crate README for the access
//! pattern and data caveats (cash-flow quarters are year-to-date, `0` can mean
//! "no data", ratio table gaps).
//!
//! ```no_run
//! use cafef::{CafefClient, PeriodType};
//!
//! # async fn demo() -> Result<(), cafef::CafefError> {
//! let client = CafefClient::new()?;
//! let docs = client.list_all_documents("VNM").await?;
//! let statements = docs.iter().filter(|d| d.is_financial_statement()).count();
//! tracing::info!(total = docs.len(), statements, "VNM library");
//!
//! let bs = client.balance_sheet("VNM", PeriodType::Year, Some(2)).await?;
//! for row in bs.rows.iter().filter(|r| r.code == "270") {
//!     for v in &row.values {
//!         tracing::info!(period = %v.period, value = ?v.value, "total assets (VND)");
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod documents;
pub mod error;
pub mod response;
pub mod statements;

pub use client::{API_BASE_URL, CafefClient, CafefClientBuilder, WWW_BASE_URL, validate_symbol};
pub use documents::{
    Assurance, Document, DocumentCategory, ShownTime, StatementScope, TitleInfo, parse_documents,
};
pub use error::CafefError;
pub use rust_decimal::Decimal;
pub use statements::{
    Period, PeriodType, PeriodValue, Statement, StatementKind, StatementPeriod, StatementRow, Unit,
    parse_statement,
};
