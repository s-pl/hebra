//! Heddle's fleet of an owner: settings, machines, decisions, simulation.

use axum::extract::{Path as UrlPath, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::St;
use crate::db::{self, Account, FleetRow};
use crate::error::{ApiError, ApiResult};
use crate::fleet::{credentials_place, providers};
use crate::identity::Principal;
use crate::server::Server;

async fn managed(s: &Server, who: &Principal, handle: &str) -> ApiResult<Account> {
    let owner =
        s.db.account_by_handle(handle)
            .await?
            .filter(|a| !a.disabled)
            .ok_or_else(|| ApiError::not_found(format!("`{handle}` not found")))?;
    if !s.manages(who, &owner, false).await? {
        return Err(ApiError::forbidden(
            "forbidden",
            format!("you do not manage `{handle}`"),
        ));
    }
    Ok(owner)
}

fn month_start() -> i64 {
    let now = jiff::Zoned::now();
    now.start_of_day()
        .ok()
        .and_then(|d| d.with().day(1).build().ok())
        .map_or(0, |d| d.timestamp().as_millisecond())
}

fn public(fleet: &FleetRow) -> Value {
    json!({
        "enabled": fleet.enabled,
        "provider": fleet.provider,
        "config": fleet.config,
        "has_credentials": fleet.credentials.is_some(),
        "updated_at": db::rfc3339(fleet.updated_at),
        "updated_by": fleet.updated_by,
    })
}

pub(crate) async fn show(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let fleet = s.db.fleet(&owner.id).await?;
    let machines = s.db.machines(&owner.id, true).await?;
    let spend: i64 = machines
        .iter()
        .filter(|m| !matches!(m.state.as_str(), "gone" | "failed"))
        .map(|m| m.price_micros)
        .sum();
    Ok(axum::Json(json!({
        "fleet": fleet.as_ref().map(public),
        "providers": providers::KINDS,
        "machines": machines,
        "events": s.db.fleet_events(&owner.id, 80).await?,
        "spend_per_hour_micros": spend,
        "cost_this_month_micros": s.db.fleet_cost_since(&owner.id, month_start()).await?,
        "queue": s.db.queued_runs(&owner.id).await?,
        "runs": s.db.fleet_runs(&owner.id, db::now_ms() - 3_600_000).await?,
    })))
}

#[derive(Deserialize)]
pub(crate) struct FleetBody {
    enabled: bool,
    provider: String,
    #[serde(default)]
    config: Value,
    /// The provider's API token; absent keeps the one stored.
    credentials: Option<String>,
}

fn validate(provider: &str, config: &Value) -> ApiResult<()> {
    if !providers::KINDS.contains(&provider) {
        return Err(ApiError::bad(format!(
            "the provider is one of {}",
            providers::KINDS.join(", ")
        )));
    }
    if !config.is_object() && !config.is_null() {
        return Err(ApiError::bad("config is an object"));
    }
    if let Some(goal) = config["goal"].as_str()
        && hebra_heddle::Goal::parse(goal).is_none()
    {
        return Err(ApiError::bad("the goal is cheap, balanced or fast"));
    }
    if config["max_machines"].as_u64().is_some_and(|n| n > 200) {
        return Err(ApiError::bad("at most 200 machines"));
    }
    if config["max_spend_per_hour"]
        .as_f64()
        .is_some_and(|x| x < 0.0)
    {
        return Err(ApiError::bad("the spending limit cannot be negative"));
    }
    if let Some(url) = config["server_url"].as_str()
        && !url.is_empty()
        && !url.starts_with("http://")
        && !url.starts_with("https://")
    {
        return Err(ApiError::bad("server_url starts with http:// or https://"));
    }
    Ok(())
}

pub(crate) async fn put(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<FleetBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let config = if body.config.is_null() {
        json!({})
    } else {
        body.config
    };
    validate(&body.provider, &config)?;
    let sealed = match body.credentials.as_deref().map(str::trim) {
        Some(token) if !token.is_empty() => Some(
            s.sealer
                .seal(&credentials_place(&owner.id), token.as_bytes())?,
        ),
        _ => None,
    };
    s.db.put_fleet(
        &owner.id,
        body.enabled,
        &body.provider,
        &config,
        sealed.as_deref(),
        &who.account.handle,
    )
    .await?;
    s.forget_fleet_catalog(&owner.id);
    s.db.fleet_event(
        &owner.id,
        "settings",
        None,
        &format!(
            "{} {} the fleet on {}",
            who.account.handle,
            if body.enabled {
                "switched on"
            } else {
                "switched off"
            },
            body.provider
        ),
    )
    .await?;
    let fleet = s.db.fleet(&owner.id).await?;
    Ok(axum::Json(json!({ "fleet": fleet.as_ref().map(public) })))
}

/// Tries the stored settings: the provider answers with its machine types.
pub(crate) async fn check(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let fleet =
        s.db.fleet(&owner.id)
            .await?
            .ok_or_else(|| ApiError::bad("set up the fleet first"))?;
    s.forget_fleet_catalog(&owner.id);
    let provider = s.fleet_provider(&fleet)?;
    let types = s.fleet_catalog(&fleet, &provider).await?;
    Ok(axum::Json(json!({ "ok": true, "types": types })))
}

#[derive(Deserialize)]
pub(crate) struct SimulateBody {
    #[serde(default = "week")]
    days: i64,
}

const fn week() -> i64 {
    7
}

pub(crate) async fn simulate(
    State(s): St,
    headers: HeaderMap,
    UrlPath(handle): UrlPath<String>,
    axum::Json(body): axum::Json<SimulateBody>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    Ok(axum::Json(s.simulate_fleet(&owner.id, body.days).await?))
}

pub(crate) async fn stop(
    State(s): St,
    headers: HeaderMap,
    UrlPath((handle, id)): UrlPath<(String, String)>,
) -> ApiResult<axum::Json<Value>> {
    let who = s.require_principal(&headers).await?;
    let owner = managed(&s, &who, &handle).await?;
    let machine =
        s.db.machine(&owner.id, &id)
            .await?
            .ok_or_else(|| ApiError::not_found("machine not found"))?;
    let fleet =
        s.db.fleet(&owner.id)
            .await?
            .ok_or_else(|| ApiError::bad("this owner has no fleet"))?;
    let provider = s.fleet_provider(&fleet)?;
    s.stop_machine(
        &provider,
        &machine,
        &format!("stopped by {}", who.account.handle),
    )
    .await?;
    Ok(axum::Json(json!({ "stopped": machine.name })))
}
