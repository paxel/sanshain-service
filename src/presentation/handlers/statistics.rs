//! The statistics page's data (san-1021): one route per tab, every answer
//! aggregated by the application layer so no person's name leaves it.

use crate::AppState;
use crate::application::statistics_service::{
    self as statistics, StatisticsAnswer, StatisticsRange, StatisticsTab,
};
use crate::domain::models::AppError;
use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Deserialize)]
pub struct StatisticsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// `GET /admin/statistics/{tab}?from=YYYY-MM-DD&to=YYYY-MM-DD`.
pub async fn admin_statistics(
    State(state): State<AppState>,
    Path(tab): Path<String>,
    Query(query): Query<StatisticsQuery>,
) -> Result<Response, AppError> {
    let today = chrono::Utc::now().date_naive();
    let range = StatisticsRange::parse(query.from.as_deref(), query.to.as_deref(), today)?;
    let Some(tab) = StatisticsTab::parse(&tab) else {
        return Err(AppError::NotFound(format!("no statistics tab '{tab}'")));
    };
    let repo = &state.repo;
    Ok(match tab {
        StatisticsTab::Landscape => answer(tab, range, statistics::landscape(repo, range).await?),
        StatisticsTab::Activity => answer(tab, range, statistics::activity(repo, range).await?),
        StatisticsTab::Versions => answer(tab, range, statistics::versions(repo, range).await?),
        StatisticsTab::Branches => answer(tab, range, statistics::branches(repo, range).await?),
        StatisticsTab::Access => answer(
            tab,
            range,
            statistics::access(repo, &chrono::Utc::now().to_rfc3339()).await?,
        ),
    })
}

fn answer<T: Serialize>(tab: StatisticsTab, range: StatisticsRange, data: T) -> Response {
    Json(StatisticsAnswer {
        tab: tab.as_str(),
        range,
        data,
    })
    .into_response()
}
