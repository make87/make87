use axum::extract::{FromRequestParts, Query};
use axum::http::{StatusCode, request::Parts};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::Deserialize;

fn de_opt_date_or_rfc3339<'de, D>(d: D) -> Result<Option<mongodb::bson::DateTime>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(d)?;
    let Some(s) = s else {
        return Ok(None);
    };

    // Try full RFC3339 first
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Ok(Some(mongodb::bson::DateTime::from_millis(
            dt.timestamp_millis(),
        )));
    }

    // Fallback: YYYY-MM-DD
    if let Ok(date) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
        let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
        return Ok(Some(mongodb::bson::DateTime::from_millis(
            dt.timestamp_millis(),
        )));
    }

    Err(serde::de::Error::custom(
        "invalid date format (expected RFC3339 or YYYY-MM-DD)",
    ))
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PaginationParams {
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<u32>,

    #[serde(default, deserialize_with = "de_opt_date_or_rfc3339")]
    since: Option<mongodb::bson::DateTime>,
    #[serde(default, deserialize_with = "de_opt_date_or_rfc3339")]
    until: Option<mongodb::bson::DateTime>,
}

#[derive(Debug, Clone)]
pub struct RequestPagination {
    pub offset: u64,
    pub limit: u32,
    pub since: Option<mongodb::bson::DateTime>,
    pub until: Option<mongodb::bson::DateTime>,
}

impl<S> FromRequestParts<S> for RequestPagination
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<PaginationParams>::from_request_parts(parts, state)
            .await
            .unwrap_or_else(|_| Query(PaginationParams::default()));

        Ok(Self {
            offset: params.offset.unwrap_or(0),
            limit: params.limit.unwrap_or(50),
            since: params.since,
            until: params.until,
        })
    }
}

impl RequestPagination {
    pub fn max_limit() -> Self {
        Self {
            offset: 0,
            limit: 100,
            since: None,
            until: None,
        }
    }
}
