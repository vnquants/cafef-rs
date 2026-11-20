//! Document library ("Tài liệu" page): models and parsing.
//!
//! Source: `GET https://cafef.vn/du-lieu/Ajax/PageNew/FileBCTC.ashx?Symbol=fpt&Type=1&Year=0`,
//! which answers `{"Data":[...],"Message":null,"Success":true}` with every row of
//! one tab at once (there is no pagination).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

use crate::error::CafefError;
use crate::response::parse_json;

/// Tab of the "Tài liệu" page a document is listed under. The tab is the only
/// document-type information CafeF gives: every row has `"Type": 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DocumentCategory {
    /// "Báo cáo tài chính" (`Type=1`): quarterly, half-year and annual financial
    /// statements, consolidated and parent-only.
    FinancialStatement,
    /// "Bản cáo bạch & BCTN" (`Type=3`): prospectuses, annual reports, charters.
    ProspectusAndAnnualReport,
    /// "Nghị quyết" (`Type=4`): board and shareholder-meeting resolutions.
    Resolution,
    /// "Báo cáo quản trị" (`Type=5`): corporate-governance reports.
    GovernanceReport,
    /// Listed under "Tất cả" (`Type=0`) but in none of the four tabs.
    /// Not seen for FPT or VNM on 2026-11-19.
    Other,
}

impl DocumentCategory {
    /// The four tabs, in page order.
    pub const TABS: [Self; 4] = [
        Self::FinancialStatement,
        Self::ProspectusAndAnnualReport,
        Self::Resolution,
        Self::GovernanceReport,
    ];

    /// Value of the `Type` query parameter. `0` lists every document ("Tất cả").
    pub fn type_param(self) -> u8 {
        match self {
            Self::FinancialStatement => 1,
            Self::ProspectusAndAnnualReport => 3,
            Self::Resolution => 4,
            Self::GovernanceReport => 5,
            Self::Other => 0,
        }
    }

    /// Tab label as shown on cafef.vn.
    pub fn label_vi(self) -> &'static str {
        match self {
            Self::FinancialStatement => "Báo cáo tài chính",
            Self::ProspectusAndAnnualReport => "Bản cáo bạch & BCTN",
            Self::Resolution => "Nghị quyết",
            Self::GovernanceReport => "Báo cáo quản trị",
            Self::Other => "Tất cả",
        }
    }

    /// Whether the server honours the `Year` filter for this tab. For
    /// resolutions and governance reports the page hides the year box and the
    /// server returns every row regardless of `Year`.
    pub fn supports_year_filter(self) -> bool {
        matches!(
            self,
            Self::FinancialStatement | Self::ProspectusAndAnnualReport | Self::Other
        )
    }
}

impl fmt::Display for DocumentCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::FinancialStatement => "financial_statement",
            Self::ProspectusAndAnnualReport => "prospectus_annual_report",
            Self::Resolution => "resolution",
            Self::GovernanceReport => "governance_report",
            Self::Other => "other",
        })
    }
}

/// The "Thời gian cập nhật" column, parsed. Despite the column header this is
/// a reporting period for financial statements and annual reports, and a
/// calendar date only for resolutions and governance reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShownTime {
    /// `"Q3/2026"`. Half-year (6-month, reviewed) statements are also filed
    /// under `Q2`.
    Quarter {
        /// Fiscal year.
        year: i32,
        /// 1 to 4.
        quarter: u8,
    },
    /// `"CN/2025"` ("cả năm"): annual document.
    FullYear {
        /// Fiscal year.
        year: i32,
    },
    /// `"27-10-2026"` (`dd-mm-yyyy`): date of the resolution or report.
    Date {
        /// Calendar date as shown (Vietnam local date).
        date: NaiveDate,
    },
    /// Anything else, for example `"CN/0"` on undated charters.
    Unknown,
}

impl ShownTime {
    /// Parse the raw `Time` string.
    ///
    /// ```
    /// use cafef::ShownTime;
    /// assert_eq!(ShownTime::parse("Q3/2026"), ShownTime::Quarter { year: 2026, quarter: 3 });
    /// assert_eq!(ShownTime::parse("CN/2025"), ShownTime::FullYear { year: 2025 });
    /// assert_eq!(ShownTime::parse("CN/0"), ShownTime::Unknown);
    /// ```
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some((left, year)) = raw.split_once('/') {
            let Some(year) = parse_year(year) else {
                return Self::Unknown;
            };
            if left.eq_ignore_ascii_case("CN") {
                return Self::FullYear { year };
            }
            if let Some(q) = left.strip_prefix(['Q', 'q'])
                && let Ok(quarter @ 1..=4) = q.parse::<u8>()
            {
                return Self::Quarter { year, quarter };
            }
            return Self::Unknown;
        }
        match NaiveDate::parse_from_str(raw, "%d-%m-%Y") {
            Ok(date) => Self::Date { date },
            Err(_) => Self::Unknown,
        }
    }

    /// Year of the period or date, if known.
    pub fn year(self) -> Option<i32> {
        use chrono::Datelike;
        match self {
            Self::Quarter { year, .. } | Self::FullYear { year } => Some(year),
            Self::Date { date } => Some(date.year()),
            Self::Unknown => None,
        }
    }
}

fn parse_year(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.len() != 4 {
        return None;
    }
    s.parse::<i32>().ok().filter(|y| *y >= 1900)
}

/// Consolidation scope read from a financial-statement title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementScope {
    /// "hợp nhất".
    Consolidated,
    /// "công ty mẹ" or "riêng": parent-only / separate statements.
    Parent,
    /// Neither word: companies without subsidiaries, and older filings.
    Unspecified,
}

/// Assurance level read from a financial-statement title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assurance {
    /// "(đã kiểm toán)": audited, normally the annual statement.
    Audited,
    /// "(đã soát xét)": reviewed, normally the half-year statement.
    Reviewed,
    /// No marker: unaudited quarterly statement.
    None,
}

/// What a financial-statement title says about the filing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TitleInfo {
    /// Consolidated, parent-only or unspecified.
    pub scope: StatementScope,
    /// Audited, reviewed or neither.
    pub assurance: Assurance,
}

impl TitleInfo {
    /// Parse a title such as
    /// `"Báo cáo tài chính hợp nhất quý 2 năm 2025 (đã soát xét)"`.
    ///
    /// ```
    /// use cafef::{Assurance, StatementScope, TitleInfo};
    /// let info = TitleInfo::parse("Báo cáo tài chính hợp nhất quý 2 năm 2025 (đã soát xét)");
    /// assert_eq!(info.scope, StatementScope::Consolidated);
    /// assert_eq!(info.assurance, Assurance::Reviewed);
    /// ```
    pub fn parse(title: &str) -> Self {
        let t = title.to_lowercase();
        let scope = if t.contains("hợp nhất") || t.contains("hop nhat") {
            StatementScope::Consolidated
        } else if t.contains("công ty mẹ") || t.contains("riêng") || t.contains("cong ty me") {
            StatementScope::Parent
        } else {
            StatementScope::Unspecified
        };
        let assurance = if t.contains("kiểm toán") || t.contains("kiem toan") {
            Assurance::Audited
        } else if t.contains("soát xét") || t.contains("soat xet") {
            Assurance::Reviewed
        } else {
            Assurance::None
        };
        Self { scope, assurance }
    }
}

/// One downloadable document from the CafeF library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Ticker in upper case, as requested.
    pub symbol: String,
    /// Tab the document was listed under.
    pub category: DocumentCategory,
    /// CafeF's record id (a 24-hex-digit MongoDB ObjectId). Present for
    /// financial statements and annual reports, `None` for resolutions and
    /// governance reports.
    pub cafef_id: Option<String>,
    /// Title as shown ("Loại báo cáo" column), whitespace collapsed.
    pub title: String,
    /// "Thời gian cập nhật" column exactly as CafeF sends it.
    pub time_raw: String,
    /// [`time_raw`](Self::time_raw) parsed.
    pub shown_time: ShownTime,
    /// Raw `Year` field (0 when the row has a date instead of a period).
    pub year: i32,
    /// Raw `Quarter` field: 1-4, 5 for full year, 0 when the row has a date.
    pub quarter: i32,
    /// Download URL on `cafefnew.mediacdn.vn`, as given (may carry `?v=`).
    pub url: String,
    /// Lower-case file extension from the URL path (`pdf`, `rar`, `zip`, ...).
    pub file_ext: Option<String>,
    /// Format icon shown in the "Định dạng" column.
    pub icon_url: Option<String>,
}

impl Document {
    /// A stable identifier for de-duplication across runs: `cafef:<ObjectId>`
    /// when CafeF gives one, otherwise `url-sha256:<hex>` of the URL without its
    /// query string (the `?v=` cache-buster can change while the file does not).
    pub fn stable_id(&self) -> String {
        match &self.cafef_id {
            Some(id) => format!("cafef:{id}"),
            None => format!("url-sha256:{}", url_hash(&self.url)),
        }
    }

    /// Year of the period or date shown, if known.
    pub fn period_year(&self) -> Option<i32> {
        self.shown_time.year()
    }

    /// Title flags (consolidated / parent, audited / reviewed). Meaningful for
    /// [`DocumentCategory::FinancialStatement`] rows.
    pub fn title_info(&self) -> TitleInfo {
        TitleInfo::parse(&self.title)
    }

    /// True for the "Báo cáo tài chính" tab.
    pub fn is_financial_statement(&self) -> bool {
        self.category == DocumentCategory::FinancialStatement
    }

    /// Creation time of CafeF's database record, decoded from the ObjectId.
    /// Not a publication date: rows migrated into the current system all carry
    /// early March 2023.
    pub fn record_created_at(&self) -> Option<DateTime<Utc>> {
        self.cafef_id.as_deref().and_then(object_id_time)
    }

    /// Time encoded in the `?v=<unix ms>` parameter of newer links. For recent
    /// filings this is when CafeF uploaded the file.
    pub fn link_version_time(&self) -> Option<DateTime<Utc>> {
        link_version_time(&self.url)
    }

    /// Last path segment of the URL, safe to use as a file name.
    pub fn file_name(&self) -> Option<String> {
        file_name_from_url(&self.url)
    }
}

/// Wire format of one row.
#[derive(Debug, Clone, Deserialize)]
struct RawDocument {
    id: Option<String>,
    #[serde(rename = "Quarter", default)]
    quarter: i32,
    #[serde(rename = "Year", default)]
    year: i32,
    #[serde(rename = "Time", default)]
    time: Option<String>,
    #[serde(rename = "Name", default)]
    name: Option<String>,
    #[serde(rename = "IconFile", default)]
    icon_file: Option<String>,
    #[serde(rename = "Link", default)]
    link: Option<String>,
}

/// Wire format of the `FileBCTC.ashx` envelope.
#[derive(Debug, Deserialize)]
struct RawFileList {
    #[serde(rename = "Data", default)]
    data: Option<Vec<RawDocument>>,
    #[serde(rename = "Message", default)]
    message: Option<String>,
    #[serde(rename = "Success", default)]
    success: bool,
}

/// Parse a `FileBCTC.ashx` response body. Rows without a link are dropped
/// (they cannot be downloaded); everything else is kept as sent.
///
/// `url` is only used in error messages.
pub fn parse_documents(
    body: &str,
    symbol: &str,
    category: DocumentCategory,
    url: &str,
) -> Result<Vec<Document>, CafefError> {
    let raw: RawFileList = parse_json(url, body)?;
    if !raw.success {
        return Err(CafefError::Api {
            url: url.to_string(),
            message: raw
                .message
                .unwrap_or_else(|| "Success=false without a message".to_string()),
        });
    }
    let symbol = symbol.trim().to_uppercase();
    Ok(raw
        .data
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| {
            let url = r.link?.trim().to_string();
            if url.is_empty() {
                return None;
            }
            let time_raw = r.time.unwrap_or_default();
            Some(Document {
                symbol: symbol.clone(),
                category,
                cafef_id: r.id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                title: crate::statements::collapse_whitespace(
                    r.name.as_deref().unwrap_or_default(),
                ),
                shown_time: ShownTime::parse(&time_raw),
                time_raw,
                year: r.year,
                quarter: r.quarter,
                file_ext: file_ext_from_url(&url),
                icon_url: r.icon_file.filter(|s| !s.is_empty()),
                url,
            })
        })
        .collect())
}

/// Strip query string and fragment.
fn url_path(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

/// Lower-case extension of the last path segment, if it looks like one.
pub fn file_ext_from_url(url: &str) -> Option<String> {
    let name = url_path(url).rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    let ok = !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| ext.to_ascii_lowercase())
}

/// Last path segment, refusing anything that could escape a directory.
pub fn file_name_from_url(url: &str) -> Option<String> {
    let name = url_path(url).rsplit('/').next()?.trim();
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['\\', '/', ':'])
        || name.chars().any(char::is_control);
    (!bad).then(|| name.to_string())
}

/// SHA-256 (hex) of the URL without query string or fragment.
pub fn url_hash(url: &str) -> String {
    let digest = Sha256::digest(url_path(url.trim()).as_bytes());
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Timestamp in the first four bytes of a MongoDB ObjectId.
pub fn object_id_time(id: &str) -> Option<DateTime<Utc>> {
    if id.len() != 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let secs = u32::from_str_radix(&id[..8], 16).ok()?;
    Utc.timestamp_opt(i64::from(secs), 0).single()
}

/// Value of a `v=<unix milliseconds>` query parameter.
pub fn link_version_time(url: &str) -> Option<DateTime<Utc>> {
    let (_, query) = url.split_once('?')?;
    let query = query.split('#').next().unwrap_or(query);
    let ms = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("v="))?
        .parse::<i64>()
        .ok()?;
    // Ignore values that are clearly not milliseconds since 2000.
    if ms < 946_684_800_000 {
        return None;
    }
    Utc.timestamp_millis_opt(ms).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn shown_time_variants() {
        assert_eq!(
            ShownTime::parse("Q2/2025"),
            ShownTime::Quarter {
                year: 2025,
                quarter: 2
            }
        );
        assert_eq!(
            ShownTime::parse(" CN/2006 "),
            ShownTime::FullYear { year: 2006 }
        );
        assert_eq!(ShownTime::parse("CN/0"), ShownTime::Unknown);
        assert_eq!(ShownTime::parse("Q5/2025"), ShownTime::Unknown);
        assert_eq!(ShownTime::parse(""), ShownTime::Unknown);
        let ShownTime::Date { date } = ShownTime::parse("27-10-2026") else {
            panic!("expected a date");
        };
        assert_eq!((date.year(), date.month(), date.day()), (2026, 10, 27));
        assert_eq!(ShownTime::parse("31-02-2026"), ShownTime::Unknown);
    }

    #[test]
    fn title_info() {
        let t = TitleInfo::parse("Báo cáo tài chính công ty mẹ năm 2025 (đã kiểm toán)");
        assert_eq!(t.scope, StatementScope::Parent);
        assert_eq!(t.assurance, Assurance::Audited);
        let t = TitleInfo::parse("Báo cáo tài chính quý 3 năm 2025");
        assert_eq!(t.scope, StatementScope::Unspecified);
        assert_eq!(t.assurance, Assurance::None);
        let t = TitleInfo::parse("BÁO CÁO TÀI CHÍNH RIÊNG QUÝ 1");
        assert_eq!(t.scope, StatementScope::Parent);
    }

    #[test]
    fn extensions_and_names() {
        let u = "https://cafefnew.mediacdn.vn/Images/Uploaded/DuLieuDownload/BCTC/FPT_x_29102026090100.pdf?v=1793239261794";
        assert_eq!(file_ext_from_url(u).as_deref(), Some("pdf"));
        assert_eq!(
            file_name_from_url(u).as_deref(),
            Some("FPT_x_29102026090100.pdf")
        );
        assert_eq!(
            file_ext_from_url("https://x/2006/FPT_06Q4_BCTC.RAR").as_deref(),
            Some("rar")
        );
        assert_eq!(file_ext_from_url("https://x/download/noext"), None);
        assert_eq!(file_name_from_url("https://x/a/.."), None);
        assert_eq!(file_name_from_url("https://x/a/"), None);
    }

    #[test]
    fn hash_ignores_query() {
        let a = url_hash("https://x/y/file.pdf?v=1");
        let b = url_hash("https://x/y/file.pdf?v=2");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, url_hash("https://x/y/other.pdf"));
        // Known SHA-256 of "abc".
        assert_eq!(
            url_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn object_id_and_link_version() {
        let t = object_id_time("6ae2a84f11738645496fb1f8").unwrap();
        assert_eq!(t.timestamp(), 0x6ae2_a84f);
        // 2026-10-29 01:58:39 UTC = 08:58:39 in Vietnam, the time stamped into
        // that document's file name (`..._29102026085839.pdf`).
        assert_eq!((t.year(), t.month(), t.day()), (2026, 10, 29));
        assert_eq!(t.to_rfc3339(), "2026-10-29T01:58:39+00:00");
        assert!(object_id_time("not-an-id").is_none());
        let v = link_version_time("https://x/f.pdf?v=1793239261794").unwrap();
        assert_eq!(v.timestamp_millis(), 1_793_239_261_794);
        assert!(link_version_time("https://x/f.pdf").is_none());
        assert!(link_version_time("https://x/f.pdf?v=12").is_none());
    }
}
