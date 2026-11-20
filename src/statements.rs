//! Statement tables ("Tài chính" page): models and parsing.
//!
//! All four tables come from `https://apiweb.cafef.vn/api/...` as JSON in one
//! of two shapes:
//!
//! - **sectioned** (balance sheet `v2/BCTC/GetReportCDKT`, cash flow
//!   `v1/BCTC/GetReportLCTT`): `templace` is a list of sections, each with its
//!   line items; `data` is a list of sections, each with the periods and their
//!   values;
//! - **flat** (income statement `v1/BCTC/GetReportDetail`, ratios
//!   `v2/BCTC/FinancialIndicators`): `templace` is the list of line items and
//!   `data` the list of periods.
//!
//! Both are turned into one [`Statement`]: periods (newest first, as sent) and
//! rows of line items with one value per period.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use crate::error::CafefError;
use crate::response::parse_json;

/// Which table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StatementKind {
    /// "Cân đối kế toán" (CDKT).
    BalanceSheet,
    /// "Kết quả kinh doanh" (KQKD).
    IncomeStatement,
    /// "Lưu chuyển tiền tệ" (LCTT), indirect method.
    CashFlow,
    /// "Chỉ số tài chính" (CSTC). Annual only.
    Ratios,
}

impl StatementKind {
    /// All tables.
    pub const ALL: [Self; 4] = [
        Self::BalanceSheet,
        Self::IncomeStatement,
        Self::CashFlow,
        Self::Ratios,
    ];

    /// CafeF's short code.
    pub fn code(self) -> &'static str {
        match self {
            Self::BalanceSheet => "CDKT",
            Self::IncomeStatement => "KQKD",
            Self::CashFlow => "LCTT",
            Self::Ratios => "CSTC",
        }
    }

    /// Parse `cdkt`/`balance`, `kqkd`/`income`, `lctt`/`cashflow`, `cstc`/`ratios`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cdkt" | "bs" | "balance" | "balance_sheet" | "balance-sheet" => {
                Some(Self::BalanceSheet)
            }
            "kqkd" | "is" | "income" | "income_statement" | "income-statement" => {
                Some(Self::IncomeStatement)
            }
            "lctt" | "cf" | "cashflow" | "cash_flow" | "cash-flow" => Some(Self::CashFlow),
            "cstc" | "ratios" | "ratio" | "indicators" => Some(Self::Ratios),
            _ => None,
        }
    }

    /// Default unit of the table's values.
    pub fn default_unit(self) -> Unit {
        match self {
            Self::Ratios => Unit::Unknown,
            _ => Unit::Vnd,
        }
    }
}

impl fmt::Display for StatementKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Period granularity (`TypeTime` query parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodType {
    /// `QUY`, "Theo quý". Cash-flow values are year-to-date, see the README.
    Quarter,
    /// `NAM`, "Theo năm".
    Year,
    /// `LUYKE`, "Lũy kế 6 tháng": first-half cumulative.
    HalfYearCumulative,
}

impl PeriodType {
    /// Value of the `TypeTime` query parameter.
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Quarter => "QUY",
            Self::Year => "NAM",
            Self::HalfYearCumulative => "LUYKE",
        }
    }

    /// Parse `quarter`/`quy`/`q`, `year`/`nam`/`y`, `half`/`luyke`/`h1`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "q" | "quarter" | "quarterly" | "quy" => Some(Self::Quarter),
            "y" | "year" | "annual" | "yearly" | "nam" => Some(Self::Year),
            "h1" | "half" | "half_year" | "luyke" | "6m" => Some(Self::HalfYearCumulative),
            _ => None,
        }
    }
}

/// A reporting period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Period {
    /// Fiscal quarter.
    Quarter {
        /// Fiscal year.
        year: i32,
        /// 1 to 4.
        quarter: u8,
    },
    /// Fiscal year.
    Year {
        /// Fiscal year.
        year: i32,
    },
    /// First six months of a fiscal year, cumulative.
    HalfYear {
        /// Fiscal year.
        year: i32,
    },
}

impl Period {
    /// Fiscal year.
    pub fn year(self) -> i32 {
        match self {
            Self::Quarter { year, .. } | Self::Year { year } | Self::HalfYear { year } => year,
        }
    }
}

impl fmt::Display for Period {
    /// `Q3/2026`, `2025`, `H1/2026`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Quarter { year, quarter } => write!(f, "Q{quarter}/{year}"),
            Self::Year { year } => write!(f, "{year}"),
            Self::HalfYear { year } => write!(f, "H1/{year}"),
        }
    }
}

/// Unit of a value as sent by CafeF (the web page divides by 10^9 or 10^6 for
/// display; the API itself is in units below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Vietnamese dong. All statement values except earnings per share.
    Vnd,
    /// Dong per share (EPS lines of the income statement).
    VndPerShare,
    /// Thousand dong per share (EPS and book value in the ratio table).
    ThousandVnd,
    /// Percent, e.g. `21.43` means 21.43 %.
    Percent,
    /// Plain multiple, e.g. P/E.
    Times,
    /// Not documented by CafeF and not inferable.
    Unknown,
}

/// Metadata of one period column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementPeriod {
    /// Parsed period.
    pub period: Period,
    /// `period.to_string()`, used as the key in [`PeriodValue::period`].
    pub label: String,
    /// `time` as sent: `Q3-2026`, `2025`.
    pub time_raw: String,
    /// CafeF's statement type code: `H` consolidated, `HK` consolidated audited,
    /// `HS` consolidated reviewed (other codes likely exist for parent-only
    /// filers).
    pub report_type: Option<String>,
    /// `content` as sent, e.g. `Đã kiểm toán`, `Đã soát xét`; `None` if empty.
    pub status: Option<String>,
}

/// One value of one line item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodValue {
    /// Period label (see [`StatementPeriod::label`]).
    pub period: String,
    /// Value in the row's [`Unit`]. `None` when CafeF sent no value for the line
    /// in that period. CafeF also sends `0` for "no data"; the web page shows
    /// both as `-`. This crate keeps `0` as `Some(0)`.
    pub value: Option<Decimal>,
}

/// One line item with its values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementRow {
    /// Section code (`TN` assets, `NV` liabilities and equity, `HDKD`, `HDDT`,
    /// `HDTC` cash-flow sections); `None` for unsectioned tables.
    pub section_code: Option<String>,
    /// Section name, e.g. `Tài sản`.
    pub section_name: Option<String>,
    /// Line code, trimmed. Balance sheet and income statement mostly use the
    /// VAS form codes (`100`, `270`, `10`, `60`), but lines added after the
    /// original form get CafeF-assigned codes (`441`-`455` on the balance
    /// sheet: biological assets, dividends payable). The cash-flow table uses
    /// CafeF's own codes (`HDKD_2`); ratios use mnemonics (`EPS`, `ROE`).
    /// Empty for heading rows such as `NGUỒN VỐN`.
    pub code: String,
    /// Line name in Vietnamese, whitespace collapsed (CafeF embeds `\n`).
    pub name: String,
    /// Indentation level (`lever` in the JSON): 1-3 on the balance sheet and
    /// cash flow. The income-statement endpoint sends the line code here
    /// instead (`lever` 10 for line 10), so do not rely on it for that table.
    pub level: Option<i32>,
    /// Display order (`number`).
    pub order: Option<i32>,
    /// Unit of [`values`](Self::values).
    pub unit: Unit,
    /// One entry per [`Statement::periods`], same order.
    pub values: Vec<PeriodValue>,
}

impl StatementRow {
    /// Value for a period label such as `Q3/2026`.
    pub fn value(&self, period_label: &str) -> Option<Decimal> {
        self.values
            .iter()
            .find(|v| v.period == period_label)
            .and_then(|v| v.value)
    }
}

/// A statement table for one symbol and period type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Statement {
    /// Ticker, upper case.
    pub symbol: String,
    /// Which table.
    pub kind: StatementKind,
    /// Granularity requested.
    pub period_type: PeriodType,
    /// Number of periods CafeF has for this table (`count`), if sent.
    pub total_periods: Option<u32>,
    /// Period columns, newest first.
    pub periods: Vec<StatementPeriod>,
    /// Line items in display order.
    pub rows: Vec<StatementRow>,
}

impl Statement {
    /// First row with this line code (codes are unique within a table).
    pub fn row(&self, code: &str) -> Option<&StatementRow> {
        self.rows.iter().find(|r| r.code == code)
    }

    /// Value of line `code` in period `period_label`.
    pub fn value(&self, code: &str, period_label: &str) -> Option<Decimal> {
        self.row(code).and_then(|r| r.value(period_label))
    }

    /// True when every period CafeF reports in `count` is present.
    pub fn is_complete(&self) -> bool {
        self.total_periods
            .is_none_or(|t| self.periods.len() >= t as usize)
    }

    /// Balance-sheet sanity check: periods where total assets (line `270`)
    /// differ from total liabilities and equity (line `440`), as
    /// `(period label, assets, sources)`. Empty for other tables, and for
    /// templates without both lines (banks use other codes).
    ///
    /// CafeF's tables do contain such gaps: on 2026-11-19 VNM's audited 2025
    /// annual column had 53,312 bn of assets against 49,515 bn of sources
    /// (retained earnings sent as 0), while its Q4/2025 column balanced.
    pub fn unbalanced_periods(&self) -> Vec<(String, Decimal, Decimal)> {
        if self.kind != StatementKind::BalanceSheet {
            return Vec::new();
        }
        self.periods
            .iter()
            .filter_map(|p| {
                let assets = self.value("270", &p.label)?;
                let sources = self.value("440", &p.label)?;
                (assets != sources).then(|| (p.label.clone(), assets, sources))
            })
            .collect()
    }

    /// Append the periods of a later page. Rows are matched by section and code;
    /// rows missing on one side get `None` values. Periods already present are
    /// skipped.
    pub fn append_page(&mut self, page: Statement) {
        let known: std::collections::HashSet<String> =
            self.periods.iter().map(|p| p.label.clone()).collect();
        let new_periods: Vec<StatementPeriod> = page
            .periods
            .into_iter()
            .filter(|p| !known.contains(&p.label))
            .collect();
        if new_periods.is_empty() {
            return;
        }
        let mut incoming: HashMap<(Option<String>, String), StatementRow> = page
            .rows
            .into_iter()
            .map(|r| ((r.section_code.clone(), r.code.clone()), r))
            .collect();
        for row in &mut self.rows {
            let key = (row.section_code.clone(), row.code.clone());
            let other = incoming.remove(&key);
            for p in &new_periods {
                let value = other.as_ref().and_then(|o| o.value(&p.label));
                row.values.push(PeriodValue {
                    period: p.label.clone(),
                    value,
                });
            }
        }
        // Rows that only exist on the later page (template changed).
        let mut extra: Vec<StatementRow> = incoming.into_values().collect();
        extra.sort_by_key(|r| r.order);
        for mut row in extra {
            row.values = self
                .periods
                .iter()
                .chain(new_periods.iter())
                .map(|p| PeriodValue {
                    period: p.label.clone(),
                    value: row.value(&p.label),
                })
                .collect();
            self.rows.push(row);
        }
        self.periods.extend(new_periods);
        debug_assert!(
            self.rows
                .iter()
                .all(|r| r.values.len() == self.periods.len())
        );
    }

    /// Keep only the newest `n` periods.
    pub fn truncate_periods(&mut self, n: usize) {
        self.periods.truncate(n);
        for row in &mut self.rows {
            row.values.truncate(n);
        }
    }
}

// ---------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<T> {
    #[serde(default)]
    is_success: bool,
    #[serde(default)]
    errors: Option<Vec<serde_json::Value>>,
    #[serde(default = "Option::default")]
    value: Option<T>,
}

#[derive(Debug, Deserialize)]
struct RawLine {
    #[serde(default)]
    lever: Option<i32>,
    #[serde(default)]
    number: Option<i32>,
    code: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCell {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    value: Option<Decimal>,
}

#[derive(Debug, Deserialize)]
struct RawPeriod {
    #[serde(default)]
    year: i32,
    #[serde(default)]
    quater: i32,
    #[serde(rename = "type", default)]
    report_type: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    data: Vec<RawCell>,
}

#[derive(Debug, Deserialize)]
struct RawSectionTemplate {
    code: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    data: Vec<RawLine>,
}

#[derive(Debug, Deserialize)]
struct RawSectionData {
    code: String,
    #[serde(default)]
    data: Vec<RawPeriod>,
}

#[derive(Debug, Deserialize)]
struct SectionedValue {
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    templace: Vec<RawSectionTemplate>,
    #[serde(default)]
    data: Vec<RawSectionData>,
}

#[derive(Debug, Deserialize)]
struct FlatValue {
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    templace: Vec<RawLine>,
    #[serde(default)]
    data: Vec<RawPeriod>,
}

/// Which JSON shape an endpoint uses.
fn is_sectioned(kind: StatementKind) -> bool {
    matches!(kind, StatementKind::BalanceSheet | StatementKind::CashFlow)
}

/// Parse one page of a statement endpoint.
///
/// `url` is only used in error messages.
pub fn parse_statement(
    body: &str,
    symbol: &str,
    kind: StatementKind,
    period_type: PeriodType,
    url: &str,
) -> Result<Statement, CafefError> {
    let symbol = symbol.trim().to_uppercase();
    if is_sectioned(kind) {
        let env: Envelope<SectionedValue> = parse_json(url, body)?;
        let value = unwrap_envelope(env, url)?;
        let sections = value
            .templace
            .into_iter()
            .map(|s| (Some((s.code, s.name.unwrap_or_default())), s.data))
            .collect();
        let periods = value
            .data
            .into_iter()
            .map(|s| (Some(s.code), s.data))
            .collect();
        Ok(build(
            symbol,
            kind,
            period_type,
            value.count,
            sections,
            periods,
        ))
    } else {
        let env: Envelope<FlatValue> = parse_json(url, body)?;
        let value = unwrap_envelope(env, url)?;
        Ok(build(
            symbol,
            kind,
            period_type,
            value.count,
            vec![(None, value.templace)],
            vec![(None, value.data)],
        ))
    }
}

fn unwrap_envelope<T>(env: Envelope<T>, url: &str) -> Result<T, CafefError> {
    let errors = env
        .errors
        .unwrap_or_default()
        .iter()
        .map(|e| e.as_str().map_or_else(|| e.to_string(), str::to_string))
        .collect::<Vec<_>>();
    if !env.is_success {
        return Err(CafefError::Api {
            url: url.to_string(),
            message: if errors.is_empty() {
                "isSuccess=false without errors".to_string()
            } else {
                errors.join("; ")
            },
        });
    }
    env.value.ok_or_else(|| CafefError::UnexpectedResponse {
        url: url.to_string(),
        reason: "isSuccess=true but no value".to_string(),
        snippet: String::new(),
    })
}

type Sections = Vec<(Option<(String, String)>, Vec<RawLine>)>;
type PeriodGroups = Vec<(Option<String>, Vec<RawPeriod>)>;

fn build(
    symbol: String,
    kind: StatementKind,
    period_type: PeriodType,
    count: Option<u32>,
    sections: Sections,
    groups: PeriodGroups,
) -> Statement {
    // Period columns in order of first appearance.
    let mut periods: Vec<StatementPeriod> = Vec::new();
    let mut period_index: HashMap<String, usize> = HashMap::new();
    // (section, line code) -> value per period index.
    let mut cells: HashMap<(Option<String>, String), Vec<Option<Decimal>>> = HashMap::new();

    for (section, raw_periods) in groups {
        let section = section.map(|s| s.trim().to_string());
        for rp in raw_periods {
            let sp = to_period(&rp, kind, period_type);
            let idx = *period_index.entry(sp.label.clone()).or_insert_with(|| {
                periods.push(sp);
                periods.len() - 1
            });
            for cell in rp.data {
                let Some(code) = cell.code else { continue };
                // Codes are matched trimmed: CafeF sends e.g. "\t450".
                let code = code.trim().to_string();
                let slot = cells.entry((section.clone(), code)).or_default();
                if slot.len() <= idx {
                    slot.resize(idx + 1, None);
                }
                // Keep the first value if a page repeats a code.
                if slot[idx].is_none() {
                    slot[idx] = cell.value;
                }
            }
        }
    }

    let mut rows = Vec::new();
    for (section, lines) in sections {
        let (section_code, section_name) = match section {
            Some((c, n)) => (
                Some(c.trim().to_string()),
                Some(collapse_whitespace(&n)).filter(|s| !s.is_empty()),
            ),
            None => (None, None),
        };
        for line in lines {
            let name = collapse_whitespace(line.name.as_deref().unwrap_or_default());
            let code = line.code.trim().to_string();
            let unit = row_unit(kind, &code, &name);
            let found = cells.get(&(section_code.clone(), code.clone()));
            let values = periods
                .iter()
                .enumerate()
                .map(|(i, p)| PeriodValue {
                    period: p.label.clone(),
                    value: found.and_then(|v| v.get(i).copied().flatten()),
                })
                .collect();
            rows.push(StatementRow {
                section_code: section_code.clone(),
                section_name: section_name.clone(),
                code,
                name,
                level: line.lever,
                order: line.number,
                unit,
                values,
            });
        }
    }

    Statement {
        symbol,
        kind,
        period_type,
        total_periods: count,
        periods,
        rows,
    }
}

fn to_period(rp: &RawPeriod, kind: StatementKind, period_type: PeriodType) -> StatementPeriod {
    let time_raw = rp.time.clone().unwrap_or_default();
    let year = if rp.year > 0 {
        rp.year
    } else {
        year_from_time(&time_raw).unwrap_or(0)
    };
    let period = match (kind, period_type) {
        (StatementKind::Ratios, _) | (_, PeriodType::Year) => Period::Year { year },
        (_, PeriodType::HalfYearCumulative) => Period::HalfYear { year },
        (_, PeriodType::Quarter) => {
            let quarter = u8::try_from(rp.quater)
                .ok()
                .filter(|q| (1..=4).contains(q))
                .or_else(|| quarter_from_time(&time_raw))
                .unwrap_or(0);
            Period::Quarter { year, quarter }
        }
    };
    StatementPeriod {
        label: period.to_string(),
        period,
        time_raw,
        report_type: rp
            .report_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        status: rp
            .content
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

/// Trim and collapse runs of whitespace (CafeF line names contain `\n` and
/// double spaces) into single spaces.
///
/// ```
/// assert_eq!(
///     cafef::statements::collapse_whitespace(" 4. Thuế phải nộp Nhà\nnước  ngắn hạn "),
///     "4. Thuế phải nộp Nhà nước ngắn hạn"
/// );
/// ```
pub fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `"Q3-2026"` -> 2026, `"2025"` -> 2025.
fn year_from_time(time: &str) -> Option<i32> {
    let digits: String = time
        .rsplit(['-', '/'])
        .next()?
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten()
}

/// `"Q3-2026"` -> 3.
fn quarter_from_time(time: &str) -> Option<u8> {
    let rest = time.trim().strip_prefix(['Q', 'q'])?;
    let q = rest.chars().next()?.to_digit(10)?;
    u8::try_from(q).ok().filter(|q| (1..=4).contains(q))
}

/// Unit of one line, following the web page's own formatting rules.
pub fn row_unit(kind: StatementKind, code: &str, name: &str) -> Unit {
    match kind {
        StatementKind::IncomeStatement => {
            // Same test as the page's `isEpsCode`: name mentions shares and the
            // code is an EPS line (70/71 for companies and insurers, 23 for banks).
            let n = name.to_lowercase();
            let eps_name = n.contains("cổ phiếu") || n.contains("co phieu");
            if eps_name && matches!(code, "70" | "71" | "23") {
                Unit::VndPerShare
            } else {
                Unit::Vnd
            }
        }
        StatementKind::BalanceSheet | StatementKind::CashFlow => Unit::Vnd,
        StatementKind::Ratios => match code.to_ascii_uppercase().as_str() {
            "EPS" | "BV" | "BVPS" => Unit::ThousandVnd,
            "ROA" | "ROE" | "ROS" | "GOS" | "DAR" | "ROEA" | "ROAA" => Unit::Percent,
            "PE" | "P/E" | "PB" | "P/B" | "TSTTHH" | "KNTTLV" => Unit::Times,
            _ => Unit::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_labels() {
        assert_eq!(
            Period::Quarter {
                year: 2026,
                quarter: 3
            }
            .to_string(),
            "Q3/2026"
        );
        assert_eq!(Period::Year { year: 2025 }.to_string(), "2025");
        assert_eq!(Period::HalfYear { year: 2026 }.to_string(), "H1/2026");
    }

    #[test]
    fn time_helpers() {
        assert_eq!(year_from_time("Q3-2026"), Some(2026));
        assert_eq!(year_from_time("2025"), Some(2025));
        assert_eq!(year_from_time(""), None);
        assert_eq!(quarter_from_time("Q3-2026"), Some(3));
        assert_eq!(quarter_from_time("2025"), None);
    }

    #[test]
    fn units() {
        assert_eq!(
            row_unit(
                StatementKind::IncomeStatement,
                "70",
                "19. Lãi cơ bản trên cổ phiếu (Đồng/1 cổ phiếu)"
            ),
            Unit::VndPerShare
        );
        assert_eq!(
            row_unit(
                StatementKind::IncomeStatement,
                "60",
                "18. Lợi nhuận sau thuế"
            ),
            Unit::Vnd
        );
        // Treasury shares on the balance sheet mention shares but are money.
        assert_eq!(
            row_unit(StatementKind::BalanceSheet, "415", "Cổ phiếu quỹ"),
            Unit::Vnd
        );
        assert_eq!(
            row_unit(StatementKind::Ratios, "EPS", ""),
            Unit::ThousandVnd
        );
        assert_eq!(row_unit(StatementKind::Ratios, "ROE", ""), Unit::Percent);
        assert_eq!(row_unit(StatementKind::Ratios, "PE", ""), Unit::Times);
        assert_eq!(row_unit(StatementKind::Ratios, "XYZ", ""), Unit::Unknown);
    }

    #[test]
    fn parse_kind_and_period_type() {
        assert_eq!(
            StatementKind::parse("CDKT"),
            Some(StatementKind::BalanceSheet)
        );
        assert_eq!(
            StatementKind::parse("income"),
            Some(StatementKind::IncomeStatement)
        );
        assert_eq!(StatementKind::parse("nope"), None);
        assert_eq!(PeriodType::parse("quy"), Some(PeriodType::Quarter));
        assert_eq!(PeriodType::parse("NAM"), Some(PeriodType::Year));
        assert_eq!(
            PeriodType::parse("luyke"),
            Some(PeriodType::HalfYearCumulative)
        );
    }

    fn page(labels: &[&str], rows: &[(&str, &[Option<i64>])]) -> Statement {
        let periods = labels
            .iter()
            .map(|l| {
                let year: i32 = l.parse().unwrap();
                StatementPeriod {
                    period: Period::Year { year },
                    label: (*l).to_string(),
                    time_raw: (*l).to_string(),
                    report_type: None,
                    status: None,
                }
            })
            .collect();
        let rows = rows
            .iter()
            .enumerate()
            .map(|(i, (code, vals))| StatementRow {
                section_code: None,
                section_name: None,
                code: (*code).to_string(),
                name: (*code).to_string(),
                level: Some(1),
                order: Some(i32::try_from(i).unwrap()),
                unit: Unit::Vnd,
                values: labels
                    .iter()
                    .zip(vals.iter())
                    .map(|(l, v)| PeriodValue {
                        period: (*l).to_string(),
                        value: v.map(Decimal::from),
                    })
                    .collect(),
            })
            .collect();
        Statement {
            symbol: "X".into(),
            kind: StatementKind::BalanceSheet,
            period_type: PeriodType::Year,
            total_periods: Some(3),
            periods,
            rows,
        }
    }

    #[test]
    fn append_and_truncate() {
        let mut a = page(&["2025", "2024"], &[("100", &[Some(1), Some(2)])]);
        let b = page(
            &["2024", "2023"],
            &[("100", &[Some(2), Some(3)]), ("200", &[Some(9), Some(8)])],
        );
        assert!(!a.is_complete());
        a.append_page(b);
        assert!(a.is_complete());
        let labels: Vec<_> = a.periods.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["2025", "2024", "2023"]);
        assert_eq!(a.value("100", "2023"), Some(Decimal::from(3)));
        let r200 = a.row("200").unwrap();
        assert_eq!(r200.values.len(), 3);
        assert_eq!(r200.value("2025"), None);
        assert_eq!(r200.value("2024"), Some(Decimal::from(9)));
        assert_eq!(r200.value("2023"), Some(Decimal::from(8)));
        a.truncate_periods(1);
        assert!(a.rows.iter().all(|r| r.values.len() == 1));
    }
}
