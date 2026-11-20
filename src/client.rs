//! HTTP client for CafeF.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};
use wreq::Client;
use wreq::header::{ACCEPT, HeaderMap, HeaderValue, ORIGIN, REFERER};
use wreq_util::{Emulation, Platform, Profile};

use crate::documents::{Document, DocumentCategory, parse_documents};
use crate::error::CafefError;
use crate::response::{check_text_response, html_title, is_challenge, looks_like_html, snippet};
use crate::statements::{PeriodType, Statement, StatementKind, parse_statement};

/// `cafef.vn`: pages and the `.ashx` document handler.
pub const WWW_BASE_URL: &str = "https://cafef.vn";
/// `apiweb.cafef.vn`: JSON API behind the statement tables.
pub const API_BASE_URL: &str = "https://apiweb.cafef.vn";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Periods per statement request. The web page asks for 4; the API accepted
/// 100 (2026-11-19), 20 keeps responses around 130 KB for the balance sheet.
const DEFAULT_STATEMENT_PAGE_SIZE: u32 = 20;
const MAX_STATEMENT_PAGE_SIZE: u32 = 100;
/// Hard stop for page walking, far above any real history (83 quarters for FPT).
const MAX_STATEMENT_PAGES: u32 = 200;

/// Builder for [`CafefClient`].
#[derive(Debug, Clone)]
pub struct CafefClientBuilder {
    timeout: Duration,
    min_interval: Duration,
    statement_page_size: u32,
    www_base_url: String,
    api_base_url: String,
}

impl Default for CafefClientBuilder {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            min_interval: DEFAULT_MIN_INTERVAL,
            statement_page_size: DEFAULT_STATEMENT_PAGE_SIZE,
            www_base_url: WWW_BASE_URL.to_string(),
            api_base_url: API_BASE_URL.to_string(),
        }
    }
}

impl CafefClientBuilder {
    /// Defaults: 30 s timeout, at most one request per second, 20 periods per
    /// statement request.
    pub fn new() -> Self {
        Self::default()
    }

    /// Per-request timeout (connect + whole response).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Minimum gap between the starts of two requests, shared by all clones
    /// of the client. Default 1 s.
    pub fn min_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    /// Same as [`min_interval`](Self::min_interval) in milliseconds.
    pub fn rate_limit_ms(self, ms: u64) -> Self {
        self.min_interval(Duration::from_millis(ms))
    }

    /// Periods requested per statement page, clamped to 1..=100.
    pub fn statement_page_size(mut self, size: u32) -> Self {
        self.statement_page_size = size.clamp(1, MAX_STATEMENT_PAGE_SIZE);
        self
    }

    /// Override both base URLs (for a proxy or a local mock). No trailing slash.
    pub fn base_urls(mut self, www: impl Into<String>, api: impl Into<String>) -> Self {
        self.www_base_url = www.into().trim_end_matches('/').to_string();
        self.api_base_url = api.into().trim_end_matches('/').to_string();
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<CafefClient, CafefError> {
        // Chrome on Windows: TLS, HTTP/2 and default headers from one profile,
        // as in vietstock-rs.
        let emulation = Emulation::builder()
            .profile(Profile::Chrome149)
            .platform(Platform::Windows)
            .http2(true)
            .build();
        let http = Client::builder()
            .emulation(emulation)
            .timeout(self.timeout)
            .redirect(wreq::redirect::Policy::limited(5))
            .build()?;
        Ok(CafefClient {
            http,
            www_base_url: self.www_base_url,
            api_base_url: self.api_base_url,
            min_interval: self.min_interval,
            statement_page_size: self.statement_page_size,
            next_slot: Arc::new(Mutex::new(None)),
        })
    }
}

/// Client for CafeF's document library and statement tables. Anonymous; no
/// session or cookies are needed. Cheap to clone; clones share the rate limit.
///
/// ```no_run
/// use cafef::{CafefClient, DocumentCategory, PeriodType};
///
/// # async fn demo() -> Result<(), cafef::CafefError> {
/// let client = CafefClient::new()?;
/// let docs = client.list_documents("FPT", DocumentCategory::FinancialStatement, None).await?;
/// tracing::info!(count = docs.len(), "financial statements");
///
/// let bs = client.balance_sheet("FPT", PeriodType::Quarter, Some(4)).await?;
/// if let Some(p) = bs.periods.first() {
///     tracing::info!(period = %p.label, total_assets = ?bs.value("270", &p.label), "VND");
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct CafefClient {
    http: Client,
    www_base_url: String,
    api_base_url: String,
    min_interval: Duration,
    statement_page_size: u32,
    next_slot: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for CafefClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CafefClient")
            .field("www_base_url", &self.www_base_url)
            .field("api_base_url", &self.api_base_url)
            .field("min_interval", &self.min_interval)
            .field("statement_page_size", &self.statement_page_size)
            .finish_non_exhaustive()
    }
}

impl CafefClient {
    /// Client with default settings.
    pub fn new() -> Result<Self, CafefError> {
        CafefClientBuilder::new().build()
    }

    /// Builder for custom timeout, rate limit and page size.
    pub fn builder() -> CafefClientBuilder {
        CafefClientBuilder::new()
    }

    // -----------------------------------------------------------------------
    // Plumbing
    // -----------------------------------------------------------------------

    /// Wait for this request's slot. Slots are reserved under the lock and
    /// slept on outside it, so concurrent callers queue up `min_interval` apart.
    async fn throttle(&self) {
        let wait = {
            let mut next = self
                .next_slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let slot = match *next {
                Some(t) if t > now => t,
                _ => now,
            };
            *next = Some(slot + self.min_interval);
            slot.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            debug!(wait_ms = wait.as_millis(), "rate limit");
            tokio::time::sleep(wait).await;
        }
    }

    fn headers(&self, json_api: bool) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        h.insert(REFERER, HeaderValue::from_static("https://cafef.vn/"));
        if json_api {
            // The page calls apiweb.cafef.vn cross-origin with fetch().
            h.insert(ORIGIN, HeaderValue::from_static("https://cafef.vn"));
        } else {
            // jQuery $.ajax on the same origin.
            h.insert(
                "x-requested-with",
                HeaderValue::from_static("XMLHttpRequest"),
            );
        }
        h
    }

    async fn get_text(&self, url: &str, json_api: bool) -> Result<String, CafefError> {
        self.throttle().await;
        let t0 = Instant::now();
        let resp = self
            .http
            .get(url)
            .headers(self.headers(json_api))
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body = resp.text().await?;
        debug!(%url, status, bytes = body.len(), elapsed_ms = t0.elapsed().as_millis(), "GET");
        check_text_response(url, status, body)
    }

    // -----------------------------------------------------------------------
    // Documents
    // -----------------------------------------------------------------------

    /// URL of the document handler for one tab.
    pub fn documents_url(
        &self,
        symbol: &str,
        category: DocumentCategory,
        year: Option<i32>,
    ) -> String {
        format!(
            "{}/du-lieu/Ajax/PageNew/FileBCTC.ashx?Symbol={}&Type={}&Year={}",
            self.www_base_url,
            symbol.to_ascii_lowercase(),
            category.type_param(),
            year.unwrap_or(0)
        )
    }

    /// List one tab of a symbol's document library in a single request (the
    /// endpoint has no pagination: `Year=0` returns the whole history).
    ///
    /// `year` is passed to the server, which honours it for financial
    /// statements and annual reports only; it is also applied locally (by the
    /// year of the shown period or date) so the result means the same for
    /// every tab. [`DocumentCategory::Other`] lists the "Tất cả" tab.
    #[tracing::instrument(skip(self), err)]
    pub async fn list_documents(
        &self,
        symbol: &str,
        category: DocumentCategory,
        year: Option<i32>,
    ) -> Result<Vec<Document>, CafefError> {
        let symbol = validate_symbol(symbol)?;
        let url = self.documents_url(&symbol, category, year);
        let body = self.get_text(&url, false).await?;
        let mut docs = parse_documents(&body, &symbol, category, &url)?;
        if let Some(y) = year {
            docs.retain(|d| d.period_year() == Some(y));
        }
        info!(%symbol, %category, ?year, count = docs.len(), "documents listed");
        Ok(docs)
    }

    /// The whole document library: the four tabs (four requests) plus the
    /// "Tất cả" list (one request) to catch rows that sit in no tab, which are
    /// returned as [`DocumentCategory::Other`]. De-duplicated by URL.
    #[tracing::instrument(skip(self), err)]
    pub async fn list_all_documents(&self, symbol: &str) -> Result<Vec<Document>, CafefError> {
        let symbol = validate_symbol(symbol)?;
        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        for category in DocumentCategory::TABS {
            for doc in self.list_documents(&symbol, category, None).await? {
                if seen.insert(doc.url.clone()) {
                    all.push(doc);
                }
            }
        }
        let everything = self
            .list_documents(&symbol, DocumentCategory::Other, None)
            .await?;
        let listed_in_all = everything.len();
        let mut extra = 0usize;
        for doc in everything {
            if seen.insert(doc.url.clone()) {
                extra += 1;
                all.push(doc);
            }
        }
        if extra > 0 {
            warn!(%symbol, extra, "documents listed under 'Tất cả' but in no tab");
        }
        info!(%symbol, count = all.len(), listed_in_all, "document library listed");
        Ok(all)
    }

    /// Download a document's bytes.
    pub async fn download_document(&self, document: &Document) -> Result<Vec<u8>, CafefError> {
        self.download_url(&document.url).await
    }

    /// Download any CafeF file URL. Fails with [`CafefError::HtmlPage`] if the
    /// CDN answers with an HTML page instead of the file.
    #[tracing::instrument(skip(self), err)]
    pub async fn download_url(&self, url: &str) -> Result<Vec<u8>, CafefError> {
        self.throttle().await;
        let t0 = Instant::now();
        let mut headers = HeaderMap::new();
        headers.insert(REFERER, HeaderValue::from_static("https://cafef.vn/"));
        let resp = self.http.get(url).headers(headers).send().await?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(wreq::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let bytes = resp.bytes().await?.to_vec();
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).into_owned();
        if content_type.starts_with("text/html") || looks_like_html(&head) {
            let title = html_title(&head);
            return Err(CafefError::HtmlPage {
                url: url.to_string(),
                status,
                challenge: is_challenge(title.as_deref(), &head),
                title,
            });
        }
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => CafefError::AccessDenied {
                    url: url.to_string(),
                    status,
                },
                _ => CafefError::Status {
                    url: url.to_string(),
                    status,
                    body: snippet(&head),
                },
            });
        }
        info!(%url, bytes = bytes.len(), %content_type, elapsed_ms = t0.elapsed().as_millis(), "downloaded");
        Ok(bytes)
    }

    /// Download a document into `dir` under the last segment of its URL.
    pub async fn download_document_to_file(
        &self,
        document: &Document,
        dir: &Path,
    ) -> Result<PathBuf, CafefError> {
        let bytes = self.download_document(document).await?;
        let name = document
            .file_name()
            .unwrap_or_else(|| format!("{}.bin", document.stable_id().replace(':', "_")));
        std::fs::create_dir_all(dir)?;
        let path = dir.join(name);
        std::fs::write(&path, &bytes)?;
        Ok(path)
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    /// URL of one statement page.
    pub fn statement_url(
        &self,
        symbol: &str,
        kind: StatementKind,
        period_type: PeriodType,
        page_index: u32,
        page_size: u32,
    ) -> String {
        let symbol = symbol.to_ascii_uppercase();
        let base = &self.api_base_url;
        let (p, tt) = (page_index.max(1), period_type.api_value());
        match kind {
            StatementKind::BalanceSheet => format!(
                "{base}/api/v2/BCTC/GetReportCDKT?symbol={symbol}&pageIndex={p}&pageSize={page_size}&reportType=ALL&TypeTime={tt}"
            ),
            StatementKind::IncomeStatement => format!(
                "{base}/api/v1/BCTC/GetReportDetail?symbol={symbol}&pageIndex={p}&pageSize={page_size}&reportType=KQKD&TypeTime={tt}"
            ),
            StatementKind::CashFlow => format!(
                "{base}/api/v1/BCTC/GetReportLCTT?symbol={symbol}&pageIndex={p}&pageSize={page_size}&reportType=ALL&TypeTime={tt}"
            ),
            StatementKind::Ratios => format!(
                "{base}/api/v2/BCTC/FinancialIndicators?symbol={symbol}&pageIndex={p}&pageSize={page_size}"
            ),
        }
    }

    /// One page of a statement table (`page_index` from 1, newest periods
    /// first). Ratios exist only per year: other period types fail with
    /// [`CafefError::Unsupported`].
    #[tracing::instrument(skip(self), err)]
    pub async fn statement_page(
        &self,
        symbol: &str,
        kind: StatementKind,
        period_type: PeriodType,
        page_index: u32,
        page_size: u32,
    ) -> Result<Statement, CafefError> {
        let symbol = validate_symbol(symbol)?;
        if kind == StatementKind::Ratios && period_type != PeriodType::Year {
            return Err(CafefError::Unsupported(
                "CafeF publishes financial ratios per year only".to_string(),
            ));
        }
        let page_size = page_size.clamp(1, MAX_STATEMENT_PAGE_SIZE);
        let url = self.statement_url(&symbol, kind, period_type, page_index, page_size);
        let body = self.get_text(&url, true).await?;
        parse_statement(&body, &symbol, kind, period_type, &url)
    }

    /// A statement table with up to `max_periods` periods (all when `None`),
    /// walking pages until CafeF's `count` is reached or a page comes back
    /// short. Logs a warning, and [`Statement::is_complete`] is false, if pages
    /// ran out before `count`.
    #[tracing::instrument(skip(self), err)]
    pub async fn statement(
        &self,
        symbol: &str,
        kind: StatementKind,
        period_type: PeriodType,
        max_periods: Option<usize>,
    ) -> Result<Statement, CafefError> {
        let t0 = Instant::now();
        let page_size = match max_periods {
            Some(n) => u32::try_from(n)
                .unwrap_or(MAX_STATEMENT_PAGE_SIZE)
                .clamp(1, self.statement_page_size),
            None => self.statement_page_size,
        };
        let want = max_periods.unwrap_or(usize::MAX);
        let mut acc = self
            .statement_page(symbol, kind, period_type, 1, page_size)
            .await?;
        let mut last_len = acc.periods.len();
        let mut page = 1;
        while acc.periods.len() < want
            && !acc.is_complete()
            && last_len >= page_size as usize
            && page < MAX_STATEMENT_PAGES
        {
            page += 1;
            let next = self
                .statement_page(symbol, kind, period_type, page, page_size)
                .await?;
            last_len = next.periods.len();
            let before = acc.periods.len();
            acc.append_page(next);
            if acc.periods.len() == before {
                break;
            }
        }
        if acc.periods.len() < want && !acc.is_complete() {
            warn!(
                symbol = %acc.symbol, %kind, ?period_type,
                got = acc.periods.len(), total = ?acc.total_periods,
                "statement pages ended before CafeF's period count"
            );
        }
        if let Some(n) = max_periods {
            acc.truncate_periods(n);
        }
        info!(
            symbol = %acc.symbol, %kind, ?period_type,
            periods = acc.periods.len(), rows = acc.rows.len(), total = ?acc.total_periods,
            pages = page, elapsed_ms = t0.elapsed().as_millis(),
            "statement fetched"
        );
        Ok(acc)
    }

    /// Balance sheet ("Cân đối kế toán").
    pub async fn balance_sheet(
        &self,
        symbol: &str,
        period_type: PeriodType,
        max_periods: Option<usize>,
    ) -> Result<Statement, CafefError> {
        self.statement(
            symbol,
            StatementKind::BalanceSheet,
            period_type,
            max_periods,
        )
        .await
    }

    /// Income statement ("Kết quả kinh doanh").
    pub async fn income_statement(
        &self,
        symbol: &str,
        period_type: PeriodType,
        max_periods: Option<usize>,
    ) -> Result<Statement, CafefError> {
        self.statement(
            symbol,
            StatementKind::IncomeStatement,
            period_type,
            max_periods,
        )
        .await
    }

    /// Cash-flow statement ("Lưu chuyển tiền tệ"). Quarterly values are
    /// year-to-date (Q2 = first half, Q3 = nine months), as in the filings.
    pub async fn cash_flow(
        &self,
        symbol: &str,
        period_type: PeriodType,
        max_periods: Option<usize>,
    ) -> Result<Statement, CafefError> {
        self.statement(symbol, StatementKind::CashFlow, period_type, max_periods)
            .await
    }

    /// Annual financial ratios ("Chỉ số tài chính").
    pub async fn ratios(
        &self,
        symbol: &str,
        max_periods: Option<usize>,
    ) -> Result<Statement, CafefError> {
        self.statement(symbol, StatementKind::Ratios, PeriodType::Year, max_periods)
            .await
    }
}

/// Trim and upper-case a ticker; 1 to 10 ASCII letters or digits.
pub fn validate_symbol(symbol: &str) -> Result<String, CafefError> {
    let s = symbol.trim();
    if s.is_empty() || s.len() > 10 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CafefError::InvalidSymbol(symbol.to_string()));
    }
    Ok(s.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        assert_eq!(validate_symbol(" fpt ").unwrap(), "FPT");
        assert!(validate_symbol("").is_err());
        assert!(validate_symbol("FPT&x=1").is_err());
        assert!(validate_symbol("ABCDEFGHIJK").is_err());
    }

    #[test]
    fn urls() {
        let c = CafefClient::new().unwrap();
        assert_eq!(
            c.documents_url("FPT", DocumentCategory::FinancialStatement, None),
            "https://cafef.vn/du-lieu/Ajax/PageNew/FileBCTC.ashx?Symbol=fpt&Type=1&Year=0"
        );
        assert_eq!(
            c.documents_url("VNM", DocumentCategory::Resolution, Some(2025)),
            "https://cafef.vn/du-lieu/Ajax/PageNew/FileBCTC.ashx?Symbol=vnm&Type=4&Year=2025"
        );
        assert_eq!(
            c.statement_url(
                "fpt",
                StatementKind::BalanceSheet,
                PeriodType::Quarter,
                1,
                4
            ),
            "https://apiweb.cafef.vn/api/v2/BCTC/GetReportCDKT?symbol=FPT&pageIndex=1&pageSize=4&reportType=ALL&TypeTime=QUY"
        );
        assert_eq!(
            c.statement_url(
                "FPT",
                StatementKind::IncomeStatement,
                PeriodType::Year,
                2,
                20
            ),
            "https://apiweb.cafef.vn/api/v1/BCTC/GetReportDetail?symbol=FPT&pageIndex=2&pageSize=20&reportType=KQKD&TypeTime=NAM"
        );
        assert_eq!(
            c.statement_url(
                "FPT",
                StatementKind::CashFlow,
                PeriodType::HalfYearCumulative,
                1,
                4
            ),
            "https://apiweb.cafef.vn/api/v1/BCTC/GetReportLCTT?symbol=FPT&pageIndex=1&pageSize=4&reportType=ALL&TypeTime=LUYKE"
        );
        assert_eq!(
            c.statement_url("FPT", StatementKind::Ratios, PeriodType::Year, 1, 4),
            "https://apiweb.cafef.vn/api/v2/BCTC/FinancialIndicators?symbol=FPT&pageIndex=1&pageSize=4"
        );
    }

    #[tokio::test]
    async fn quarterly_ratios_are_unsupported_without_network() {
        let c = CafefClient::new().unwrap();
        let err = c
            .statement_page("FPT", StatementKind::Ratios, PeriodType::Quarter, 1, 4)
            .await
            .unwrap_err();
        assert!(matches!(err, CafefError::Unsupported(_)));
    }

    #[tokio::test]
    async fn throttle_spaces_requests() {
        let c = CafefClient::builder()
            .min_interval(Duration::from_millis(50))
            .build()
            .unwrap();
        let t0 = Instant::now();
        c.throttle().await;
        c.throttle().await;
        c.throttle().await;
        assert!(t0.elapsed() >= Duration::from_millis(100));
    }
}
