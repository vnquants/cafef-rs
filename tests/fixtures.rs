//! Deserialization tests against trimmed responses captured from CafeF on
//! 2026-11-19 (FPT). No network.

use cafef::response::check_text_response;
use cafef::{
    Assurance, CafefError, Decimal, DocumentCategory, Period, PeriodType, ShownTime, StatementKind,
    StatementScope, Unit, parse_documents, parse_statement,
};
use chrono::Datelike;

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn dec(n: i64) -> Option<Decimal> {
    Some(Decimal::from(n))
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

#[test]
fn financial_statement_documents() {
    let body = fixture("documents_fpt_financial_statements.json");
    let docs = parse_documents(&body, "fpt", DocumentCategory::FinancialStatement, "u").unwrap();
    assert_eq!(docs.len(), 7);
    assert!(docs.iter().all(|d| d.symbol == "FPT"));
    assert!(docs.iter().all(|d| d.is_financial_statement()));

    let first = &docs[0];
    assert_eq!(first.cafef_id.as_deref(), Some("6ae2a84f11738645496fb1f8"));
    assert_eq!(first.stable_id(), "cafef:6ae2a84f11738645496fb1f8");
    assert_eq!(first.title, "Báo cáo tài chính công ty mẹ quý 3 năm 2026");
    assert_eq!(first.time_raw, "Q3/2026");
    assert_eq!(
        first.shown_time,
        ShownTime::Quarter {
            year: 2026,
            quarter: 3
        }
    );
    assert_eq!((first.year, first.quarter), (2026, 3));
    assert_eq!(first.file_ext.as_deref(), Some("pdf"));
    assert_eq!(first.title_info().scope, StatementScope::Parent);
    assert_eq!(first.title_info().assurance, Assurance::None);
    assert_eq!(
        first.link_version_time().unwrap().timestamp_millis(),
        1_793_239_120_240
    );
    assert_eq!(first.record_created_at().unwrap().day(), 29);
    assert!(first.url.contains("?v="));
    assert!(first.file_name().unwrap().ends_with(".pdf"));

    // Half-year reviewed consolidated statement is filed under Q2.
    let reviewed = &docs[2];
    assert_eq!(reviewed.time_raw, "Q2/2026");
    assert_eq!(reviewed.title_info().scope, StatementScope::Consolidated);
    assert_eq!(reviewed.title_info().assurance, Assurance::Reviewed);

    // Annual audited: Quarter = 5 on the wire, "CN/2025" shown.
    let annual = &docs[3];
    assert_eq!(annual.quarter, 5);
    assert_eq!(annual.shown_time, ShownTime::FullYear { year: 2025 });
    assert_eq!(annual.title_info().assurance, Assurance::Audited);
    assert!(annual.link_version_time().is_none());

    // Old filings are .rar archives with migration-era record ids (March 2023).
    let rar = &docs[5];
    assert_eq!(rar.file_ext.as_deref(), Some("rar"));
    assert_eq!(rar.period_year(), Some(2008));
    let created = rar.record_created_at().unwrap();
    assert_eq!((created.year(), created.month()), (2023, 3));

    let oldest = &docs[6];
    assert_eq!(oldest.shown_time, ShownTime::FullYear { year: 2005 });
}

#[test]
fn annual_report_documents() {
    let body = fixture("documents_fpt_annual_reports.json");
    let docs = parse_documents(
        &body,
        "FPT",
        DocumentCategory::ProspectusAndAnnualReport,
        "u",
    )
    .unwrap();
    assert_eq!(docs.len(), 3);
    assert_eq!(docs[1].title, "Báo cáo thường niên năm 2025");
    assert_eq!(docs[1].shown_time, ShownTime::FullYear { year: 2025 });
    // Undated charter: "CN/0", Year 0.
    assert_eq!(docs[2].time_raw, "CN/0");
    assert_eq!(docs[2].shown_time, ShownTime::Unknown);
    assert_eq!(docs[2].period_year(), None);
}

#[test]
fn resolution_documents_have_dates_and_no_ids() {
    let body = fixture("documents_fpt_resolutions.json");
    let docs = parse_documents(&body, "FPT", DocumentCategory::Resolution, "u").unwrap();
    assert_eq!(docs.len(), 2);
    let d = &docs[0];
    assert_eq!(d.cafef_id, None);
    assert_eq!((d.year, d.quarter), (0, 0));
    let ShownTime::Date { date } = d.shown_time else {
        panic!("expected a date, got {:?}", d.shown_time);
    };
    assert_eq!((date.year(), date.month(), date.day()), (2026, 10, 27));
    assert_eq!(d.period_year(), Some(2026));
    // Hash of the URL without its ?v= cache-buster.
    let id = d.stable_id();
    assert!(id.starts_with("url-sha256:"), "{id}");
    assert_eq!(id.len(), "url-sha256:".len() + 64);
    let mut moved = d.clone();
    moved.url = moved.url.replace("v=1793185860557", "v=1");
    assert_eq!(moved.stable_id(), id);
    assert_ne!(docs[1].stable_id(), id);
}

#[test]
fn governance_documents() {
    let body = fixture("documents_fpt_governance.json");
    let docs = parse_documents(&body, "FPT", DocumentCategory::GovernanceReport, "u").unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].title, "Báo cáo tình hình quản trị công ty");
    assert!(docs.iter().all(|d| d.file_ext.as_deref() == Some("pdf")));
    assert!(matches!(docs[1].shown_time, ShownTime::Date { .. }));
}

#[test]
fn documents_success_false_is_an_api_error() {
    let body = fixture("documents_error_success_false.json");
    let err = parse_documents(&body, "FPT", DocumentCategory::Other, "u").unwrap_err();
    let CafefError::Api { message, .. } = err else {
        panic!("expected Api error, got {err:?}");
    };
    assert!(message.contains("Object reference"));
}

#[test]
fn documents_round_trip_through_serde() {
    let body = fixture("documents_fpt_financial_statements.json");
    let docs = parse_documents(&body, "FPT", DocumentCategory::FinancialStatement, "u").unwrap();
    let json = serde_json::to_string(&docs).unwrap();
    let back: Vec<cafef::Document> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, docs);
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

#[test]
fn balance_sheet_quarterly() {
    let body = fixture("balance_sheet_fpt_quarter.json");
    let st = parse_statement(
        &body,
        "fpt",
        StatementKind::BalanceSheet,
        PeriodType::Quarter,
        "u",
    )
    .unwrap();
    assert_eq!(st.symbol, "FPT");
    assert_eq!(st.total_periods, Some(83));
    let labels: Vec<_> = st.periods.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["Q3/2026", "Q2/2026"]);
    assert_eq!(
        st.periods[0].period,
        Period::Quarter {
            year: 2026,
            quarter: 3
        }
    );
    assert_eq!(st.periods[0].time_raw, "Q3-2026");
    assert_eq!(st.periods[0].report_type.as_deref(), Some("H"));
    assert_eq!(st.periods[0].status, None);

    assert_eq!(st.rows.len(), 11);
    // CafeF sends the code "\t450" (biological assets, a CafeF-assigned code);
    // codes are trimmed on both sides so the values still match.
    let bio = st.row("450").unwrap();
    assert_eq!(bio.name, "V. Tài sản sinh học ngắn hạn");
    assert_eq!(bio.value("Q3/2026"), dec(0));
    // Names lose embedded newlines.
    let tax = st.row("313").unwrap();
    assert_eq!(tax.name, "4. Thuế và các khoản phải nộp Nhà nước ngăn hạn");
    assert_eq!(tax.value("Q3/2026"), dec(1_545_131_912_654));
    // Header row without a code.
    let header = st.row("").unwrap();
    assert_eq!(header.name, "NGUỒN VỐN");
    assert_eq!(header.section_code.as_deref(), Some("NV"));
    let total = st.row("270").unwrap();
    assert_eq!(total.name, "TỔNG CỘNG TÀI SẢN");
    assert_eq!(total.section_code.as_deref(), Some("TN"));
    assert_eq!(total.section_name.as_deref(), Some("Tài sản"));
    assert_eq!(total.unit, Unit::Vnd);
    assert_eq!(total.values[0].period, "Q3/2026");
    assert_eq!(total.values[0].value, dec(75_329_492_718_303));
    assert_eq!(st.value("270", "Q2/2026"), dec(73_734_163_085_489));
    // Assets = liabilities + equity.
    assert_eq!(st.value("440", "Q3/2026"), st.value("270", "Q3/2026"));
    assert_eq!(st.row("440").unwrap().section_code.as_deref(), Some("NV"));
    assert!(st.rows.iter().all(|r| r.values.len() == 2));
    assert!(!st.is_complete());
    assert!(st.unbalanced_periods().is_empty());

    // A broken column (as seen for VNM 2025) is reported.
    let mut broken = st.clone();
    let row = broken.rows.iter_mut().find(|r| r.code == "440").unwrap();
    row.values[0].value = dec(1);
    let bad = broken.unbalanced_periods();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].0, "Q3/2026");
}

#[test]
fn balance_sheet_annual_is_audited() {
    let body = fixture("balance_sheet_fpt_year.json");
    let st = parse_statement(
        &body,
        "FPT",
        StatementKind::BalanceSheet,
        PeriodType::Year,
        "u",
    )
    .unwrap();
    assert_eq!(st.total_periods, Some(21));
    let p = &st.periods[0];
    assert_eq!(p.period, Period::Year { year: 2025 });
    assert_eq!(p.label, "2025");
    assert_eq!(p.report_type.as_deref(), Some("HK"));
    assert_eq!(p.status.as_deref(), Some("Đã kiểm toán"));
    assert_eq!(st.value("270", "2025"), dec(88_141_991_634_625));
    assert_eq!(st.value("111", "2024"), dec(6_725_619_929_289));
}

#[test]
fn income_statement_quarterly() {
    let body = fixture("income_statement_fpt_quarter.json");
    let st = parse_statement(
        &body,
        "FPT",
        StatementKind::IncomeStatement,
        PeriodType::Quarter,
        "u",
    )
    .unwrap();
    assert_eq!(st.rows.len(), 24);
    assert!(st.rows.iter().all(|r| r.section_code.is_none()));
    let revenue = st.row("10").unwrap();
    assert_eq!(
        revenue.name,
        "3. Doanh thu thuần về bán hàng và cung cấp dịch vụ"
    );
    assert_eq!(revenue.level, Some(10));
    assert_eq!(revenue.value("Q3/2026"), dec(13_754_159_003_395));
    assert_eq!(st.value("61", "Q2/2026"), dec(2_567_654_036_428));
    let eps = st.row("70").unwrap();
    assert_eq!(eps.unit, Unit::VndPerShare);
    assert_eq!(eps.value("Q3/2026"), dec(1174));
    assert_eq!(st.row("60").unwrap().unit, Unit::Vnd);
    // Negative values survive.
    assert!(st.value("52", "Q3/2026").unwrap() < Decimal::ZERO);
}

#[test]
fn income_statement_half_year_cumulative() {
    let body = fixture("income_statement_fpt_half_year.json");
    let st = parse_statement(
        &body,
        "FPT",
        StatementKind::IncomeStatement,
        PeriodType::HalfYearCumulative,
        "u",
    )
    .unwrap();
    let p = &st.periods[0];
    assert_eq!(p.period, Period::HalfYear { year: 2026 });
    assert_eq!(p.label, "H1/2026");
    assert_eq!(p.time_raw, "2026");
    assert_eq!(p.report_type.as_deref(), Some("HS"));
    assert_eq!(p.status.as_deref(), Some("Đã soát xét"));
    assert_eq!(st.value("10", "H1/2026"), dec(26_268_500_667_974));
}

#[test]
fn cash_flow_quarterly_is_year_to_date() {
    let body = fixture("cash_flow_fpt_quarter.json");
    let st = parse_statement(
        &body,
        "FPT",
        StatementKind::CashFlow,
        PeriodType::Quarter,
        "u",
    )
    .unwrap();
    assert_eq!(st.rows.len(), 7);
    let sections: Vec<_> = st
        .rows
        .iter()
        .filter_map(|r| r.section_code.as_deref())
        .collect();
    assert_eq!(sections[0], "HDKD");
    assert!(sections.contains(&"HDDT") && sections.contains(&"HDTC"));
    let pbt = st.row("HDKD_2").unwrap();
    // Trailing spaces in CafeF's names are trimmed.
    assert_eq!(pbt.name, "1. Lợi nhuận trước thuế");
    // Q3 value is nine-month cumulative (Q1+Q2+Q3 pre-tax profit).
    assert_eq!(pbt.value("Q3/2026"), dec(8_770_160_739_433));
    assert_eq!(pbt.value("Q2/2026"), dec(5_714_320_093_652));
    assert_eq!(st.value("HDDT_29", "Q3/2026"), dec(-5_543_350_707_262));
    // Closing cash equals balance-sheet line 110 for the same quarter.
    assert_eq!(st.value("HDTC_42", "Q3/2026"), dec(7_377_342_139_505));
}

#[test]
fn ratios_annual() {
    let body = fixture("ratios_fpt_year.json");
    let st = parse_statement(&body, "FPT", StatementKind::Ratios, PeriodType::Year, "u").unwrap();
    assert_eq!(st.total_periods, Some(24));
    assert_eq!(st.rows.len(), 12);
    let labels: Vec<_> = st.periods.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["2025", "2024"]);
    let roe = st.row("ROE").unwrap();
    assert_eq!(roe.unit, Unit::Percent);
    assert_eq!(roe.value("2025"), Some(Decimal::new(2143, 2)));
    let eps = st.row("EPS").unwrap();
    assert_eq!(eps.unit, Unit::ThousandVnd);
    assert_eq!(eps.value("2024"), Some(Decimal::new(494, 2)));
    assert_eq!(st.row("PE").unwrap().unit, Unit::Times);
    assert_eq!(st.row("TSNVTVCSH").unwrap().unit, Unit::Unknown);
}

#[test]
fn statement_serde_round_trip() {
    let body = fixture("balance_sheet_fpt_quarter.json");
    let st = parse_statement(
        &body,
        "FPT",
        StatementKind::BalanceSheet,
        PeriodType::Quarter,
        "u",
    )
    .unwrap();
    let json = serde_json::to_string(&st).unwrap();
    let back: cafef::Statement = serde_json::from_str(&json).unwrap();
    assert_eq!(back, st);
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn html_redirect_page_is_reported_as_html() {
    let body = fixture("cafef_object_moved.html");
    let err = check_text_response("u", 302, body.clone()).unwrap_err();
    let CafefError::HtmlPage {
        status,
        title,
        challenge,
        ..
    } = err
    else {
        panic!("expected HtmlPage, got {err:?}");
    };
    assert_eq!(status, 302);
    assert_eq!(title.as_deref(), Some("Object moved"));
    assert!(!challenge);
    // Same body served with 200 to a JSON parser.
    let err = parse_documents(&body, "FPT", DocumentCategory::FinancialStatement, "u").unwrap_err();
    assert!(matches!(err, CafefError::HtmlPage { .. }));
}

#[test]
fn validation_error_is_invalid_request() {
    let body = fixture("apiweb_400_problem.json");
    let err = check_text_response("u", 400, body).unwrap_err();
    let CafefError::InvalidRequest { message, .. } = err else {
        panic!("expected InvalidRequest, got {err:?}");
    };
    assert!(message.contains("TypeTime"), "{message}");
}

#[test]
fn statement_is_success_false() {
    let body = r#"{"isSuccess":false,"value":null,"errors":["symbol not found"]}"#;
    let err = parse_statement(
        body,
        "FPT",
        StatementKind::BalanceSheet,
        PeriodType::Year,
        "u",
    )
    .unwrap_err();
    let CafefError::Api { message, .. } = err else {
        panic!("expected Api error, got {err:?}");
    };
    assert_eq!(message, "symbol not found");
}
