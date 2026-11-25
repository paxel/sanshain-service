//! The statistics page's data (san-1021): one route per tab, every answer
//! aggregated by the application layer so no person's name leaves it.

use crate::AppState;
use crate::application::statistics_service::{StatisticsRange, StatisticsTab};
use crate::domain::models::AppError;
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct StatisticsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// `GET /admin/statistics/{tab}?from=YYYY-MM-DD&to=YYYY-MM-DD`.
pub async fn admin_statistics(
    State(_state): State<AppState>,
    Path(tab): Path<String>,
    Query(query): Query<StatisticsQuery>,
) -> Result<Response, AppError> {
    let today = chrono::Utc::now().date_naive();
    let range = StatisticsRange::parse(query.from.as_deref(), query.to.as_deref(), today)?;
    let Some(tab) = StatisticsTab::parse(&tab) else {
        return Err(AppError::NotFound(format!("no statistics tab '{tab}'")));
    };
    tracing::debug!(?range, ?tab, "statistics");
    match tab {}
}
