//! Live smoke test against cafef.vn. Ignored by default; run with
//! `cargo test -p cafef-rs --test live -- --ignored --nocapture`.
//! Makes 5 requests, one per second.

use cafef::{CafefClient, DocumentCategory, PeriodType, StatementKind, Unit};

#[tokio::test]
#[ignore = "network: hits cafef.vn and apiweb.cafef.vn"]
async fn live_smoke_fpt() {
    let client = CafefClient::new().unwrap();

    let docs = client
        .list_documents("FPT", DocumentCategory::FinancialStatement, None)
        .await
        .unwrap();
    assert!(docs.len() > 100, "FPT had 207 statements on 2026-11-19");
    assert!(docs.iter().all(|d| !d.url.is_empty()));
    assert!(docs.iter().any(|d| d.file_ext.as_deref() == Some("pdf")));

    let resolutions = client
        .list_documents("FPT", DocumentCategory::Resolution, None)
        .await
        .unwrap();
    assert!(!resolutions.is_empty());
    assert!(
        resolutions.iter().all(
            |d| d.stable_id().starts_with("url-sha256:") || d.stable_id().starts_with("cafef:")
        )
    );

    let bs = client
        .statement_page(
            "FPT",
            StatementKind::BalanceSheet,
            PeriodType::Quarter,
            1,
            2,
        )
        .await
        .unwrap();
    assert_eq!(bs.periods.len(), 2);
    let p = &bs.periods[0].label;
    assert_eq!(bs.value("270", p), bs.value("440", p), "assets = sources");
    assert!(bs.value("270", p).unwrap() > cafef::Decimal::ZERO);

    let is = client
        .statement_page(
            "FPT",
            StatementKind::IncomeStatement,
            PeriodType::Year,
            1,
            1,
        )
        .await
        .unwrap();
    assert_eq!(is.row("70").unwrap().unit, Unit::VndPerShare);

    let ratios = client.ratios("FPT", Some(1)).await.unwrap();
    assert_eq!(ratios.periods.len(), 1);
    assert!(ratios.row("ROE").is_some());
}
