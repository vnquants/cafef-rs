# cafef-rs

Rust client for two parts of [CafeF](https://cafef.vn)'s company data, both
readable anonymously:

- the **document library** ("Tài liệu" page, e.g.
  `https://cafef.vn/du-lieu/hose/fpt-tai-lieu.chn`): financial-statement files
  (PDF, older RAR/ZIP, some DOC/XLS), annual reports, prospectuses, charters,
  resolutions and governance reports, with download links;
- the **statement tables** ("Tài chính" page, e.g.
  `https://cafef.vn/du-lieu/hose/fpt-tai-chinh.chn`): balance sheet, income
  statement and cash flow by quarter, year and first-half cumulative, plus
  annual ratios.

Package `cafef-rs`, library `cafef`. Companion crate next to [`vietstock-rs`](https://github.com/vnquants/vietstock-rs).

| Property | Value |
|---|---|
| Hosts | `https://cafef.vn` (documents), `https://apiweb.cafef.vn` (tables), `https://cafefnew.mediacdn.vn` (files) |
| Login | Not needed for anything in this crate |
| Session / cookies / tokens | None needed |
| Pagination | Documents: none (one request returns a whole tab). Tables: `pageIndex` / `pageSize` over periods |
| Values | `rust_decimal::Decimal`, in dong as sent by the API |
| Rate limit | 1 request per second by default, shared by clones of the client |

## Quickstart

```rust
use cafef::{CafefClient, DocumentCategory, PeriodType};

#[tokio::main]
async fn main() -> Result<(), cafef::CafefError> {
    let client = CafefClient::new()?;

    // Whole library: 4 tabs + the "Tất cả" list, 5 requests.
    let docs = client.list_all_documents("VNM").await?;
    for d in docs.iter().filter(|d| d.is_financial_statement()).take(3) {
        tracing::info!(id = %d.stable_id(), title = %d.title, shown = %d.time_raw, url = %d.url, "filing");
    }
    let bytes = client.download_document(&docs[0]).await?;

    // Latest 4 quarterly balance sheets; values in dong.
    let bs = client.balance_sheet("VNM", PeriodType::Quarter, Some(4)).await?;
    let q = &bs.periods[0].label; // "Q3/2026"
    tracing::info!(total_assets = ?bs.value("270", q), "VND");
    assert!(bs.unbalanced_periods().is_empty());
    Ok(())
}
```

Examples (all log through `tracing`; `RUST_LOG=cafef=debug` shows each request):

```sh
cargo run -p cafef-rs --example list_documents -- FPT --all            # whole library, summary
cargo run -p cafef-rs --example list_documents -- VNM --year 2025      # statements tab, one year
cargo run -p cafef-rs --example get_statement -- FPT cdkt quarter 4    # balance sheet, 4 quarters
cargo run -p cafef-rs --example get_statement -- VNM kqkd year 2
cargo run -p cafef-rs --example get_statement -- FPT lctt half 1 --summary
cargo run -p cafef-rs --example get_statement -- FPT cstc year 5       # ratios
cargo run -p cafef-rs --example get_statement -- FPT cdkt quarter all --summary  # whole history
cargo run -p cafef-rs --example download_document -- FPT ./out         # newest consolidated filing
```

Tests: `cargo test -p cafef-rs` (fixtures, no network);
`cargo test -p cafef-rs --test live -- --ignored` (5 live requests).

## Access pattern (verified 2026-11-19)

How the site behaves today and how this crate deals with it. Re-check this
section when something breaks; the site changes without notice. Endpoints were
found by reading the pages' inline JavaScript and confirmed by rendering the
pages in the Obscura headless browser (the page's own calls are listed below)
and with plain HTTP.

**Transport.** [`wreq`](https://crates.io/crates/wreq) with the same Chrome 149
/ Windows TLS + HTTP/2 profile as `vietstock-rs`. CafeF does not need it today
(plain `curl` without a User-Agent got HTTP 200 from every endpoint); it is kept
as insurance and for consistency. No headless browser at runtime.

**Encodings.** Unlike `finance.vietstock.vn`, CafeF never sent raw deflate:

- `cafef.vn` answers `gzip` when gzip is offered (also when `deflate` is offered
  too) and identity when only `deflate` is offered;
- `apiweb.cafef.vn` sends uncompressed JSON whatever is offered (the full
  quarterly balance-sheet history is about 540 KB in one response).

The crate still enables only gzip/br/zstd decoding and does not advertise
`deflate`, as in `vietstock-rs`.

**Headers.** None are required. The crate imitates the page: `Referer:
https://cafef.vn/` everywhere, `X-Requested-With: XMLHttpRequest` on `cafef.vn`
(jQuery `$.ajax`), `Origin: https://cafef.vn` on `apiweb.cafef.vn` (cross-origin
`fetch`; the API answers `Access-Control-Allow-Origin: *`). `cafef.vn` pages set
an `ASP.NET_SessionId` cookie, which the endpoints below do not need.

### Endpoints

| What | Method and URL | Parameters | Returns |
|---|---|---|---|
| Document list | `GET https://cafef.vn/du-lieu/Ajax/PageNew/FileBCTC.ashx` | `Symbol` (any case, page uses lower case), `Type` (tab, below), `Year` (`0` = all) | `text/plain` JSON `{"Data":[…],"Message":null,"Success":true}`, every row of the tab at once |
| Balance sheet | `GET https://apiweb.cafef.vn/api/v2/BCTC/GetReportCDKT` | `symbol`, `pageIndex` (from 1), `pageSize`, `reportType=ALL` (or `TN` / `NV`), `TypeTime` | sectioned JSON, sections `TN` (assets) and `NV` (liabilities and equity), 133 lines |
| Income statement | `GET https://apiweb.cafef.vn/api/v1/BCTC/GetReportDetail` | same, `reportType=KQKD` | flat JSON, 24 lines. Other `reportType` values return the KQKD template with zeros |
| Cash flow (indirect) | `GET https://apiweb.cafef.vn/api/v1/BCTC/GetReportLCTT` | same, `reportType=ALL` (or `HDKD` / `HDDT` / `HDTC`) | sectioned JSON, 39 lines |
| Ratios | `GET https://apiweb.cafef.vn/api/v2/BCTC/FinancialIndicators` | `symbol`, `pageIndex`, `pageSize` (no `TypeTime`: annual only) | flat JSON, 12 indicators |
| Files | `GET https://cafefnew.mediacdn.vn/...` (the `Link` of a row) | none (`?v=` is a cache-buster) | the file (`application/pdf`, `application/x-rar-compressed`, …) |

`TypeTime` is `QUY` (quarter), `NAM` (year) or `LUYKE` (first-half cumulative);
anything else is HTTP 400 with ASP.NET problem details. The page requests
`pageSize=4`; the API accepted `pageSize=100` (all 83 FPT quarters in one
response). The crate asks for 20 periods per request by default and walks pages
until the `count` field (periods available) is reached.

Document tabs (`Type`): `1` Báo cáo tài chính, `3` Bản cáo bạch & BCTN, `4` Nghị
quyết, `5` Báo cáo quản trị, `0` Tất cả (exactly the union of 1/3/4/5 for FPT and
VNM). `Type=2` or `6` returns `Success:false` with `Object reference not set to
an instance of an object.` `Year` is honoured for tabs 1 and 3 (tab 3 also
returns its undated `CN/0` rows) and ignored for 4 and 5; the crate applies the
year filter locally as well so it means the same for every tab.

Document row fields: `id` (MongoDB ObjectId, `null` on tabs 4 and 5), `Type`
(always `0`), `Quarter` (1-4, 5 = full year, 0 for dated rows), `Year`, `Time`,
`Name`, `IconFile`, `Link`.

Table JSON: `{"isSuccess":true,"value":{"count":83,"templace":[…],"data":[…]},"errors":[]}`.
Each period carries `year`, `quater`, `time` (`Q3-2026`, `2025`), `type` (`H`
consolidated, `HK` consolidated audited, `HS` consolidated reviewed) and
`content` (`Đã kiểm toán`, `Đã soát xét` or empty); each cell is `{code, value}`.

Calls made by the "Tài chính" page itself (Obscura, FPT, balance-sheet tab):
`GetReportSummary?…&reportType=ALL&TypeTime=QUY` (the summary tab),
`/du-lieu/Ajax/FinancialAjax.aspx?isBank=false&isInsurance=false&tab=candoi&symbol=fpt`
(HTML fragment carrying the script for each tab), `GetReportCDKT?…&pageSize=4…`,
`FinancialIndicators?…&pageSize=4`. Bank tickers load the same endpoints
(`FinancialAjax.aspx?isBank=true…` embeds the same URLs).

### Errors and anti-bot behaviour

| Situation | Response | Crate error |
|---|---|---|
| Unknown symbol, tables | HTTP 500, empty body | `CafefError::Status` |
| Unknown symbol, documents | `{"Data":[],"Success":true}` | empty list |
| Unknown `.ashx` handler | HTTP 302 to `/404.aspx`, small HTML page | `CafefError::HtmlPage` |
| Bad `TypeTime` | HTTP 400 `application/problem+json` | `CafefError::InvalidRequest` |
| `Success:false` / `isSuccess:false` | JSON envelope | `CafefError::Api` |
| Any HTML where JSON was expected (error page, Cloudflare "Just a moment…", captcha) | HTML | `CafefError::HtmlPage { challenge }` |
| HTTP 401/403 | – | `CafefError::AccessDenied` |
| Quarterly ratios | not offered by CafeF | `CafefError::Unsupported` |

No challenge, captcha, rate limiting or blocking was seen at one request per
second (about 80 requests over the session, plus one rendered page load).

### Available anonymously

| Data | Anonymous | Notes |
|---|---|---|
| Document library, full history | Yes | FPT 376 files (2005-2026), VNM 322 |
| File downloads | Yes | no Referer or cookie needed |
| Balance sheet / income statement / cash flow, quarter, year, H1 | Yes | FPT 83 quarters (Q1/2006-Q3/2026), 21 years (2005-2025); VNM 87 quarters, 21 years |
| Ratios | Yes, annual only | FPT 24 years |
| Parent-only (separate) statement tables | Not found | the tables are one consolidated series (`H`/`HK`/`HS`); separate statements exist only as files |
| Summary tab (`GetReportSummary`), plan targets (`KeHoachKinhDoanh`), ratio charts (`FinancialIndicatorsChart`), securities-firm cash-flow and insurer income-statement add-ons (`/du-lieu/Ajax/PageNew/GetSecurityLcttDetail.ashx`, `GetInsuranceKqkdDetail.ashx`) | Reachable, not wrapped | not needed for statements cross-checking |

## Data caveats

**Documents.**

- The "Thời gian cập nhật" column is not an update time. For financial
  statements and annual reports it is the reporting period (`Q3/2026`,
  `CN/2025` = full year, `CN/0` = undated charter); for resolutions and
  governance reports it is the document date (`27-10-2026`). Half-year
  (reviewed) statements are filed under `Q2`. Parsed into `ShownTime`; the raw
  string is kept in `time_raw`.
- No publication date is given. Two proxies: `link_version_time()` decodes
  `?v=<unix ms>` on newer links (for FPT's Q3/2026 statement it equals the
  upload time stamped in the file name); `record_created_at()` decodes the
  ObjectId, which is early March 2023 for every row migrated from the old
  system. Record your own first-seen time for point-in-time work.
- `stable_id()` is `cafef:<ObjectId>` when CafeF gives an id (tabs 1 and 3),
  otherwise `url-sha256:<hex>` of the URL without its query string.
- Titles can be wrong: FPT's row "Báo cáo tài chính quý 3 năm 2025" (no scope
  word) links to `…BCTC_cong_ty_me_Quy_3_2025…`, i.e. the parent-only
  statement. `title_info()` only reads the title.
- File types: FPT statements 200 PDF, 6 RAR, 1 ZIP (archives are 2006-2008
  filings); VNM statements 204 PDF, 9 DOC, 3 RAR, 2 XLS, 1 XLSX. The `IconFile`
  shows a RAR icon for ZIP files too; use `file_ext`.
- The same charter can appear twice under different paths
  (`…/BCTC/FPT_BDL_2026.pdf` and `…/BanCaoBach/FPT_BDL_2026.pdf`).

**Statement tables.**

- Units: every statement value is in dong except the EPS lines of the income
  statement (codes 70/71, dong per share). Ratios: EPS and BVPS in thousand
  dong, ROE/ROA/ROS/GOS/DAR in percent, P/E and P/B as multiples. Recorded per
  row as `Unit`. The web page divides by 10^9 ("Tỷ đồng", default) or 10^6.
- `0` means "no data" as often as zero: the page shows both as `-`. The crate
  keeps `Some(0)`; `None` means the line was absent from the response.
- **`-1` is an explicit "no data" sentinel.** HPG's Q3/2016 cash flow has 35 of
  42 cells equal to exactly `-1`. In a store of 8 tickers (VNM, FPT, HPG, MWG,
  TTF, JVC, ITA, ROS), 14,730 cells were exactly `-1`, mostly in 2015-2016
  periods, which is 18 % of the non-zero cells. The crate keeps the value as
  sent (`Some(-1)`). Consumers should read it as missing: a statement that is
  almost all `-1` still passes identity checks (`-1 = -1 + -1 + -1` is within
  any tolerance). Downstream consumers (e.g. `vnquants-fundamentals`) should normalize this.
- **Cash-flow quarters are year-to-date**, as in the filings: FPT's Q2/2026
  pre-tax profit in the cash-flow table is Q1+Q2 (5,714 bn) and Q3/2026 is nine
  months (8,770 bn); opening cash is the start of the year in every quarter.
  Derive single quarters by differencing.
- One value per period; whether CafeF rewrites old periods after a
  restatement is unknown (no history is exposed). Quarterly and annual series
  are independent: FPT total assets Q4/2025 = 88,089.62 bn (quarterly report)
  vs 2025 = 88,141.99 bn (audited annual). The period's `status` /
  `report_type` say which version an annual or half-year column is.
- The tables have errors. VNM's audited 2025 annual balance sheet has total
  assets 53,312.34 bn but total liabilities and equity 49,514.74 bn (retained
  earnings sent as 0), while its Q4/2025 column balances at 53,312.37 bn.
  Older FPT quarters show differences of a few dong and one of 2.75 bn
  (Q4/2015). `Statement::unbalanced_periods()` flags these; cross-check with the
  filing PDFs before trusting a number.
- Line codes are trimmed and names whitespace-collapsed (CafeF sends `"\t450"`
  and names with embedded newlines). Balance-sheet codes follow the VAS form
  except CafeF-assigned 441-455 (biological assets, dividends payable, …); the
  heading `NGUỒN VỐN` has an empty code. Cash-flow codes are CafeF's own
  (`HDKD_2`, `HDTC_42`). The income statement's `level` is a copy of the line
  code, not an indentation level.
- Ratios are thin: FPT has no 2022 row; P/B, current ratio, interest coverage
  and debt/equity are 0 in every FPT year and in the VNM years checked
  (2023-2025); ROS equals GOS for FPT in 2021 and every earlier year (a CafeF
  bug); VNM 2025 GOS/ROS are 0 and its 2025 ROE jumps to 30.67 % (25.96 % in
  2024), plausibly because of the broken 2025 equity figure above.
- Banks, insurers and securities firms get different templates from the same
  endpoints (not checked line by line; EPS detection follows the page: code 23
  for banks).

## Live check (2026-11-19)

| | FPT | VNM |
|---|---|---|
| Library total (5 requests) | 376 | 322 |
| Financial statements | 207 (consolidated 103, parent 87, audited 36, reviewed 30), 2005-2026 | 219 (consolidated 106, parent 98, audited 37, reviewed 36), 2005-2026 |
| Annual reports, charters, prospectus | 30 | 19 |
| Resolutions / governance reports | 117 / 22 | 54 / 30 |
| Rows without CafeF id | 139 | 84 |
| Quarterly / annual balance-sheet periods | 83 / 21 | 87 / 21 |
| Total assets, latest quarter (bn) | Q3/2026 75,329.49 | Q3/2026 57,892.39 |
| Total assets, latest year (bn) | 2025 88,141.99 | 2025 53,312.34 (sources 49,514.74, see above) |

Cross-check with the rendered web page (Obscura, FPT balance-sheet tab,
"Tỷ đồng"): Q3/2026 total assets 75,329.49, liabilities 31,290.11, equity
44,039.38, cash 7,377.34; Q2/2026 total assets 73,734.16. Identical to the
crate's values divided by 10^9. Cash-flow closing cash Q3/2026 (7,377.34 bn)
equals balance-sheet line 110.
